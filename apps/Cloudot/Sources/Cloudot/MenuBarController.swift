import AppKit
import SwiftUI

/// 自持的菜单栏项 + 面板 + 主窗口。
///
/// 为什么仍然不用 `MenuBarExtra`：原来的理由（逐帧动画必须自己往 `button.image` 换图）
/// 已经不成立了，图标现在是静态 SF Symbol。**别再拿「动画到不了状态栏」当依据。**
/// 留着自建 status item 是因为还有三件事非拿到 status item 本身不可：
///   1. 同步结果那 0.7 秒的临时换图，要能精确控制什么时候换回来；
///   2. tooltip 与辅助功能标签跟着 `headline` 走；
///   3. 点击时顺手拉一次 status（`MenuBarExtra` 给不了点击回调）。
/// 迁移过去收益为零、风险不为零，所以不动。
///
/// 面板用标准 `NSPopover`（`.transient` 自带点击外部收起 + 原生毛玻璃圆角），
/// 不自建 NSPanel —— 那套是为了修 macOS 15 的面板 resize 闪烁，我们没有那个需求。
@MainActor
final class MenuBarController: NSObject {
    private let model: AppModel
    private let statusItem: NSStatusItem
    private let popover = NSPopover()

    /// 结果反馈的回退定时器。非 nil 就代表「正在显示成功/失败图标」，
    /// 期间不让状态变化把它顶掉。
    private var pulseRevert: Timer?
    private var lastPulseID = 0
    /// 上一次观察到的冲突 id。只在 nil → 有值时拉主窗口，避免用户关掉窗口后
    /// 每次状态刷新又被强制顶到前面。
    private var lastConflictID: String?

    /// 成功/失败图标停留多久再回到常驻符号。
    private static let pulseDuration: TimeInterval = 0.7

    private var mainWindow: NSWindow?

    init(model: AppModel) {
        self.model = model
        // 图标是单个 SF Symbol，squareLength 让它和系统菜单栏项对齐同宽。
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        super.init()

        configureStatusItem()
        configurePopover()
        observeModel()
        applyResting()
    }

    // MARK: - 装配

    private func configureStatusItem() {
        guard let button = statusItem.button else { return }
        // 走标准的 button.image 路径（而不是往 button 里塞 NSHostingView 子视图）：
        // 标准图路径能拿到系统对非活跃屏幕的自动变淡，与原生 App 一致。
        button.imagePosition = .imageOnly
        button.target = self
        button.action = #selector(togglePanel)
        button.toolTip = "Cloudot"
    }

    private func configurePopover() {
        let host = NSHostingController(
            rootView: MenuBarPanel(model: model, openMainWindow: { [weak self] in
                self?.showMainWindow()
            })
        )
        // 让 popover 跟着 SwiftUI 内容的固有尺寸走
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
        popover.behavior = .transient   // 点击外部自动收起，不用自己装事件监听
    }

    // MARK: - 图标

    /// 盯住状态与一次性事件。`withObservationTracking` 的 onChange 只触发一次，
    /// 所以每次都要重新武装。
    private func observeModel() {
        withObservationTracking {
            _ = model.iconState
            _ = model.pulse
            _ = model.pending
            _ = model.conflict
        } onChange: { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                self.observeModel()
                self.react()
            }
        }
    }

    private func react() {
        // 图标只有几种形状，说不清具体在发生什么 —— 那些话由 tooltip 和辅助功能标签说。
        statusItem.button?.toolTip = "Cloudot · \(model.headline)"
        statusItem.button?.setAccessibilityLabel("Cloudot · \(model.headline)")
        diag("state=\(model.iconState) pulse=\(model.pulse.map { "\($0.kind)#\($0.id)" } ?? "nil") pulsing=\(pulseRevert != nil)")

        // 确认框只在这里呈现一次（见 PendingActionPresenter）。
        if model.pending != nil {
            PendingActionPresenter.presentIfNeeded(model: model)
        }

        // 冲突 sheet 挂在主窗口上：菜单栏同步撞车时把主窗口拉起来让用户看 diff。
        // 只在冲突**新出现**时拉一次，不然用户关掉主窗口后每次 refresh 都会再被顶开。
        let conflictID = model.conflict?.id
        if let conflictID, conflictID != lastConflictID {
            lastConflictID = conflictID
            showMainWindow()
        } else if conflictID == nil {
            lastConflictID = nil
        }

        if let pulse = model.pulse, pulse.id != lastPulseID {
            lastPulseID = pulse.id
            diag("显示结果反馈 \(pulse.kind)")
            showPulse(pulse.kind)
            return
        }
        // 结果反馈期间不让状态变化把图标顶掉 —— `perform()` 结尾必然跟一次 refresh，
        // 那一轮状态变化正好落在这 0.7 秒里，不挡住就等于没有反馈。
        guard pulseRevert == nil else { return }
        applyResting()
    }

    private func applyResting() {
        setImage(MenuBarIcon.image(for: model.iconState))
    }

    /// 同步成功/失败后短暂换成结果图标，然后回到常驻符号。
    ///
    /// 这里**故意没有**「减少动态效果」判断：换图是离散的状态指示，没有位移、没有插值、
    /// 没有缓动，`accessibilityDisplayShouldReduceMotion` 管不到它；而且原来开着该开关时
    /// 走的就是这条路径（「减少动态效果」不等于「删除结果反馈」），行为上没有退步。
    /// **以后要是给菜单栏加回任何动画，这个判断必须一起加回来。**
    private func showPulse(_ kind: AppModel.IconPulse.Kind) {
        setImage(MenuBarIcon.image(for: kind))
        pulseRevert?.invalidate()
        let timer = Timer(timeInterval: Self.pulseDuration, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.pulseRevert = nil
                self.applyResting()
            }
        }
        // `.common` 而不是默认模式：面板/菜单打开时 run loop 会进入 tracking 模式，
        // default 模式的 timer 直接停摆 —— 结果图标会**永久**卡在菜单栏上。
        RunLoop.main.add(timer, forMode: .common)
        pulseRevert = timer
    }

    private func setImage(_ image: NSImage?) {
        guard let image else {
            // 只可能是符号名写错、或者系统版本比部署目标还低。保留上一张图，
            // 总比菜单栏留一块看不见但点得到的空白强。测试会让这条走不到。
            diag("符号解析失败，保留上一张图")
            return
        }
        statusItem.button?.image = image
    }

    /// 菜单栏状态诊断日志。`CLOUDOT_DIAG=1` 才输出 ——
    /// 菜单栏只能靠眼睛验收，出问题时没有日志会很难查。
    private static let diagnosticsEnabled =
        ProcessInfo.processInfo.environment["CLOUDOT_DIAG"] != nil

    private func diag(_ message: @autoclosure () -> String) {
        guard Self.diagnosticsEnabled else { return }
        NSLog("cloudot-menubar: %@", message())
    }

    // MARK: - 面板与主窗口

    @objc private func togglePanel() {
        if popover.isShown {
            popover.performClose(nil)
            return
        }
        showPopoverAnchored()
        // 面板上就有「立即同步」按钮，显示旧状态会自相矛盾（写着「已同步」还让你点同步）。
        // 只拉 status：面板用不到 doctor/apps/backups，而 doctor 单独就要 87ms。
        //
        // **不要**在 refresh 后无条件关开 popover 重锚 —— preferredContentSize 会自己
        // 跟着内容长，关开一次就闪一次。只有真的漂到屏幕角落才抢救（见 reanchorIfDrifted）。
        Task {
            await model.refreshStatusOnly()
            self.reanchorIfDrifted()
        }
    }

    /// 贴着 status item 按钮弹出。锚点无效时绝不硬 show —— 否则 popover 会掉到 (0,0)。
    private func showPopoverAnchored() {
        guard let button = statusItem.button,
              button.window != nil,
              button.bounds.width > 0, button.bounds.height > 0
        else {
            diag("status item 按钮还没就绪，跳过弹出")
            return
        }
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        // 快捷键（⌘S / ⌘Q）需要 key window；window 尚未建好时不要硬 makeKey
        if let window = popover.contentViewController?.view.window {
            window.makeKey()
        }
    }

    /// 锚点丢失时 popover 会掉到屏幕左下角附近。只在这种异常位置才关开重锚，
    /// 常态尺寸变化交给 `preferredContentSize`，避免每次点开都闪一下。
    private func reanchorIfDrifted() {
        guard popover.isShown else { return }
        guard let window = popover.contentViewController?.view.window else { return }
        let origin = window.frame.origin
        // 正常贴在菜单栏下方时 y 接近屏幕顶部；漂到 (0,0) 一带才是锚丢了。
        guard origin.x < 16, origin.y < 16 else { return }
        diag("popover 漂到 (\(origin.x), \(origin.y))，重锚")
        guard let button = statusItem.button, button.window != nil else { return }
        popover.performClose(nil)
        showPopoverAnchored()
    }

    func showMainWindow() {
        popover.performClose(nil)

        if mainWindow == nil {
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 900, height: 640),
                styleMask: [
                    .titled,
                    .closable,
                    .miniaturizable,
                    .resizable,
                    .fullSizeContentView,
                ],
                backing: .buffered,
                defer: false
            )
            window.title = "cloudot"
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.titlebarSeparatorStyle = .none
            window.toolbarStyle = .unified
            window.minSize = NSSize(
                width: CloudotTheme.windowMinimumWidth,
                height: CloudotTheme.windowMinimumHeight
            )
            window.contentViewController = NSHostingController(rootView: MainWindow(model: model))
            // 关窗只是隐藏；下次还从菜单栏打开同一个窗口
            window.isReleasedWhenClosed = false
            window.center()
            mainWindow = window
        }

        mainWindow?.makeKeyAndOrderFront(nil)
        // LSUIElement 应用需要显式激活，窗口才会到前面来
        NSApp.activate(ignoringOtherApps: true)
    }
}
