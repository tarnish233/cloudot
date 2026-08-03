import Foundation

/// 一次更新检查的结果。
struct UpdateCheck: Equatable {
    let current: SemanticVersion
    let latest: SemanticVersion
    /// DMG 的下载地址。由版本号拼出来，不查 GitHub API（见 `Updater` 里的理由）。
    let downloadURL: URL
    /// 出问题时让用户自己去看的页面 —— 比如那个版本忘传 DMG 了。
    let releasePageURL: URL

    var isAvailable: Bool { current < latest }
}
