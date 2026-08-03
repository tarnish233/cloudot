import AppKit

/// 用 AppKit `NSAlert` 呈现 `PendingAction`，全局只此一处。
///
/// 以前 MenuBarPanel 和 MainWindow 各挂一份 SwiftUI `.alert`：主窗
/// `isReleasedWhenClosed = false`，关窗后 view tree 仍在，一点确认就弹两个窗，
/// 没锚点的那个会掉到屏幕左下角。LSUIElement 下 NSAlert 比双 SwiftUI alert 稳。
@MainActor
enum PendingActionPresenter {
    /// 正在显示时不为 nil，避免 observation 重入把同一条 pending 弹两次。
    private static var isShowing = false

    static func presentIfNeeded(model: AppModel) {
        guard !isShowing, let action = model.pending else { return }
        isShowing = true

        let alert = NSAlert()
        alert.messageText = action.title
        alert.informativeText = action.explanation
        alert.alertStyle = action.isDestructive ? .warning : .informational

        let confirm = alert.addButton(withTitle: action.confirmLabel)
        if action.isDestructive {
            confirm.hasDestructiveAction = true
        }
        alert.addButton(withTitle: "取消")

        // 菜单栏工具没有自己的 key window 时，alert 仍应到前台
        NSApp.activate(ignoringOtherApps: true)
        let response = alert.runModal()

        isShowing = false
        // 用户可能在 alert 期间又点了别的入口；只清理我们弹出来的那一条
        if model.pending?.id == action.id {
            model.pending = nil
        }

        if response == .alertFirstButtonReturn {
            Task {
                await model.confirm(action)
            }
        }
    }
}
