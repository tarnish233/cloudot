import AppKit

/// 把符号名变成菜单栏能直接用的 template 图。
///
/// 单独拆出来只有一个理由：**能测**。`MenuBarController` 一构造就往真实菜单栏插一个
/// item，测试里不能碰它；而符号名写错、或者某个符号在部署目标上根本不存在，编译期
/// 完全看不出来，运行时表现成「菜单栏空了一块」—— 一个不报错、只会消失的故障。
@MainActor
enum MenuBarIcon {
    private static let configuration = NSImage.SymbolConfiguration(
        pointSize: CloudotTheme.menuBarSymbolPointSize,
        weight: .regular
    )

    static func image(for state: AppModel.IconState) -> NSImage? {
        image(symbol: state.symbol)
    }

    static func image(for pulse: AppModel.IconPulse.Kind) -> NSImage? {
        image(symbol: pulse.symbol)
    }

    static func image(symbol: String) -> NSImage? {
        guard let base = NSImage(systemSymbolName: symbol, accessibilityDescription: "Cloudot") else {
            return nil
        }
        // `withSymbolConfiguration` 返回的是**另一个实例**，`isTemplate` 和
        // `accessibilityDescription` 不保证跟过来，所以显式写死，别指望默认值 ——
        // 非 template 的固定色图在反色的菜单栏上会直接隐身。
        let configured = base.withSymbolConfiguration(configuration) ?? base
        configured.isTemplate = true
        configured.accessibilityDescription = "Cloudot"
        return configured
    }
}
