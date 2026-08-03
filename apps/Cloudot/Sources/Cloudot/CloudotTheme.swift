import Foundation

enum CloudotTheme {
    static let windowMinimumWidth = 720.0
    static let windowMinimumHeight = 500.0
    static let sidebarMinimumWidth = 184.0
    static let sidebarIdealWidth = 210.0
    static let sidebarMaximumWidth = 260.0
    static let pagePadding = 20.0
    static let sectionSpacing = 14.0
    static let compactSpacing = 8.0
    static let cardPadding = 12.0
    static let menuWidth = 340.0
    static let menuContentMaximumHeight = 320.0
    /// 菜单栏 SF Symbol 的字号。菜单栏可用高度约 18pt，15pt 的符号和系统自带项目视觉等重。
    /// 调大之前先看 `icloud.slash`：它是这批里最宽的，squareLength 会先裁到它。
    static let menuBarSymbolPointSize = 15.0
}
