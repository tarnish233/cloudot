import AppKit
import SwiftUI

/// 自持的菜单栏项 + 面板 + 主窗口。
///
/// 为什么不用 `MenuBarExtra`：它自己持有 `NSStatusItem` 且不对外暴露，而机器人的
/// 扫描、机械臂和表情动画必须自己往 `button.image` 逐帧换图才能生效。
///
/// 面板用标准 `NSPopover`（`.transient` 自带点击外部收起 + 原生毛玻璃圆角），
/// 不自建 NSPanel —— 那套是为了修 macOS 15 的面板 resize 闪烁，我们没有那个需求。
@MainActor
final class MenuBarController: NSObject {
    private let model: AppModel
    private let statusItem: NSStatusItem
    private let popover = NSPopover()

    /// 动画播放状态。动效是**展示层**的事，所以都放在这里而不是 model 里。
    private var sequence: RobotAnimationSequence?
    private var frameIndex = 0
    private var timer: Timer?
    /// 一次性动效播放中：期间不让状态变化打断它。
    private var playingOneShot = false
    private var lastPulseID = 0

    private var mainWindow: NSWindow?

    init(model: AppModel) {
        self.model = model
        // 机器人横向包含耳朵与机械臂，squareLength 会把它裁掉。
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
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

    // MARK: - 动效

    /// 盯住状态与一次性事件。`withObservationTracking` 的 onChange 只触发一次，
    /// 所以每次都要重新武装。
    private func observeModel() {
        withObservationTracking {
            _ = model.iconState
            _ = model.pulse
        } onChange: { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                self.observeModel()
                self.react()
            }
        }
    }

    private func react() {
        statusItem.button?.toolTip = "Cloudot · \(model.headline)"
        diag("state=\(model.iconState) pulse=\(model.pulse.map { "\($0.kind)#\($0.id)" } ?? "nil") oneShot=\(playingOneShot)")
        if let pulse = model.pulse, pulse.id != lastPulseID {
            lastPulseID = pulse.id
            diag("播放一次性动效 \(pulse.kind)")
            play(RobotAnimator.oneShot(for: pulse.kind), oneShot: true)
            return
        }
        guard !playingOneShot else { return }
        applyResting()
    }

    private func applyResting() {
        play(RobotAnimator.resting(for: model.iconState), oneShot: false)
    }

    private func play(_ seq: RobotAnimationSequence, oneShot: Bool) {
        timer?.invalidate()
        timer = nil
        sequence = seq
        frameIndex = 0
        playingOneShot = oneShot
        statusItem.button?.image = seq.frames.first

        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            guard oneShot, seq.frames.count > 1 else {
                playingOneShot = false
                return
            }

            // “减少动态效果”不等于“删除结果反馈”：显示动画中点的笑脸/叉眼
            // 作为静态替代，短暂停留后再回到常驻姿态。
            statusItem.button?.image = seq.frames[seq.frames.count / 2]
            let t = Timer(timeInterval: 0.7, repeats: false) { [weak self] _ in
                MainActor.assumeIsolated {
                    self?.playingOneShot = false
                    self?.applyResting()
                }
            }
            RunLoop.main.add(t, forMode: .common)
            timer = t
            return
        }

        // 单帧的静止姿态不需要定时器
        guard seq.frames.count > 1 else { return }
        let t = Timer(timeInterval: seq.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.step() }
        }
        // 不允许合并，否则帧间距忽长忽短，看着就是卡
        t.tolerance = 0
        // `.common` 而不是默认模式：面板/菜单打开时 run loop 会进入 tracking 模式，
        // default 模式的 timer 直接停摆 —— 那正好长得像「点了同步图标就卡住」。
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    private func step() {
        guard let seq = sequence else { return }
        frameIndex += 1
        if frameIndex >= seq.frames.count {
            guard seq.loops else {
                timer?.invalidate()
                timer = nil
                if playingOneShot {
                    playingOneShot = false
                    applyResting()   // 一次性动效播完，回到当前状态的常驻姿态
                }
                return
            }
            frameIndex = 0
        }
        statusItem.button?.image = seq.frames[frameIndex]
    }

    /// 动效诊断日志。`CLOUDOT_DIAG=1` 才输出 ——
    /// 菜单栏动效只能靠眼睛验收，出问题时没有日志会很难查。
    private static let diagnosticsEnabled =
        ProcessInfo.processInfo.environment["CLOUDOT_DIAG"] != nil

    private func diag(_ message: @autoclosure () -> String) {
        guard Self.diagnosticsEnabled else { return }
        NSLog("cloudot-robot: %@", message())
    }

    // MARK: - 面板与主窗口

    @objc private func togglePanel() {
        if popover.isShown {
            popover.performClose(nil)
            return
        }
        guard let button = statusItem.button else { return }
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        // 让面板里按钮上的快捷键（⌘S / ⌘Q）能生效
        popover.contentViewController?.view.window?.makeKey()
        // 面板上就有「立即同步」按钮，显示旧状态会自相矛盾（写着「已同步」还让你点同步）。
        // 只拉 status：面板用不到 doctor/apps/backups，而 doctor 单独就要 87ms。
        Task {
            await model.refreshStatusOnly()
        }
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
