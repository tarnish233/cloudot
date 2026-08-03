/// 菜单栏图标用的 SF Symbol。
///
/// 与 `Level.symbol` 分开：那套是 App 内部用的，可以配色、可以用 fill 变体压出层次；
/// 菜单栏是单色 template 渲染，颜色不生效，状态只能靠**形状**传达。所以常驻姿态一律
/// 用线条变体，只有「需要你处理」的那种才用 fill 加重。
extension AppModel.IconState {
    var symbol: String {
        switch self {
        // 常态就是 App 图标那个双箭头同步环 —— 同一个符号，菜单栏和 Finder 里
        // 认的是同一个形状。改这里之前先看 Icon/make-icon.swift，两边要一起改。
        //
        // 刷新和同步也共用它：静态图标本来就区分不了这两者，而且**在跑**这件事
        // 由面板里的 `ProgressView` 和跟着 `headline` 走的 tooltip 来说，
        // 菜单栏不必再表一次态。真正需要图标变形的只有「要你动手」和「出事了」。
        case .healthy, .refreshing, .syncing: "arrow.triangle.2.circlepath"
        case .pending: "arrow.up.arrow.down"
        case .broken: "exclamationmark.triangle.fill"
        case .unavailable: "icloud.slash"
        }
    }
}

extension AppModel.IconPulse.Kind {
    /// 结果反馈。停留 0.7 秒再落回常驻姿态，所以要和所有常驻符号都不一样 ——
    /// 撞了脸那 0.7 秒就等于没发生。
    var symbol: String {
        switch self {
        case .success: "checkmark.circle.fill"
        case .failure: "xmark.octagon.fill"
        }
    }
}
