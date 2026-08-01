import SwiftUI

/// 菜单栏弹出面板：一眼看清状态 + 一键同步。详情都留给主窗口。
struct MenuBarPanel: View {
    let model: AppModel
    /// 由 MenuBarController 注入 —— `@Environment(\.openWindow)` 在
    /// `NSHostingController` 里是空的，拿不到 SwiftUI 的 Scene 环境。
    let openMainWindow: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            MenuBarHeader(model: model)
            MenuBarContent(model: model)
            MenuBarFooter(model: model, openMainWindow: openMainWindow)
        }
        .frame(width: CloudotTheme.menuWidth)
        .tint(SystemTheme.accentColor)
        .confirm(model)
    }
}
