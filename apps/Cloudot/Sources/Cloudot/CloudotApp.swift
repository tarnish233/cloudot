import AppKit
import SwiftUI

/// 整个界面都挂在 AppDelegate 上，SwiftUI 的 Scene 只留一个占位。
///
/// 原因是菜单栏图标动画必须自己往 `NSStatusItem.button.image` 逐帧换图 ——
/// SwiftUI 的动画到不了状态栏（实测确认过）。而 `MenuBarExtra` 自己持有 status item
/// 且不对外暴露，所以只能自建。既然 status item 归我们管，面板和主窗口也一并用
/// AppKit 承载，避免再踩「SwiftUI Scene 环境在自建窗口里拿不到」的坑 ——
/// 比如 `@Environment(\.openWindow)` 在 `NSHostingController` 里就是空的。
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let model = AppModel()
    private var menuBar: MenuBarController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        menuBar = MenuBarController(model: model)
        model.startAutoRefresh()
    }

    /// 关掉主窗口不退出 —— 这是个常驻菜单栏工具。
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }
}

@main
struct CloudotApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        // App 协议要求至少有一个 Scene。界面全在 AppDelegate 里，这个永远不会显示。
        Settings { EmptyView() }
    }
}

extension Level {
    /// App 内部用的符号（可以配色，不受菜单栏单色限制）。
    var symbol: String {
        switch self {
        case .ok: "checkmark.circle.fill"
        case .warn: "arrow.triangle.2.circlepath"
        case .error: "exclamationmark.triangle.fill"
        }
    }

    var tint: Color {
        switch self {
        case .ok: SystemTheme.accentColor
        case .warn: .orange
        case .error: .red
        }
    }

    var accessibilityLabel: String {
        switch self {
        case .ok: "正常"
        case .warn: "警告"
        case .error: "错误"
        }
    }
}
