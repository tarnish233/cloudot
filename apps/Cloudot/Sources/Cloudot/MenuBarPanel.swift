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
        // 确认框与冲突面板都不在这里挂：双 presenter 会弹两个窗、锚点丢了就
        // 掉到左下角。确认走 MenuBarController 的 NSAlert；冲突 sheet 挂主窗口，
        // 同步撞车时由 Controller 自动把主窗口拉起来。
    }
}
