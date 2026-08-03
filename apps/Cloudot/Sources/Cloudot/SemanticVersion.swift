import Foundation

/// 语义化版本号，用来判断「有没有新版」。
///
/// **不能用字符串比较**：`"0.10.0" < "0.9.0"` 在字符串比较下为真，
/// 于是 0.9.0 会被当成比 0.10.0 新，用户永远收不到更新提示。
struct SemanticVersion: Comparable, CustomStringConvertible {
    private let parts: [Int]

    /// 解析失败返回 nil。宽容处理三件事：
    ///   · `v` 前缀 —— git tag 是 `v0.3.0`，Info.plist 里是 `0.3.0`，两边要能比
    ///   · 位数不同 —— `0.2` 和 `0.2.0` 视为同一个版本
    ///   · 预发布后缀 —— `0.3.0-beta.1` 砍成 `0.3.0`
    ///
    /// 砍掉后缀意味着**预发布版会被视为等于正式版**。cloudot 目前不发预发布版；
    /// 哪天要发，这里得单独处理排序，别指望现在这段是对的。
    init?(_ text: String) {
        var trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.hasPrefix("v") { trimmed.removeFirst() }
        if let dash = trimmed.firstIndex(of: "-") {
            trimmed = String(trimmed[trimmed.startIndex..<dash])
        }
        // omittingEmptySubsequences: false 让 "1..2" 产生一个空段，Int() 返回 nil，
        // 从而整体判为非法 —— 而不是悄悄解析成 [1, 2]。
        let numbers = trimmed
            .split(separator: ".", omittingEmptySubsequences: false)
            .map { Int($0) }
        guard !numbers.isEmpty, !numbers.contains(nil) else { return nil }
        parts = numbers.map { $0! }
    }

    static func < (lhs: Self, rhs: Self) -> Bool {
        // 位数不同时短的那边补 0，这样 0.2 == 0.2.0
        for index in 0..<max(lhs.parts.count, rhs.parts.count) {
            let left = index < lhs.parts.count ? lhs.parts[index] : 0
            let right = index < rhs.parts.count ? rhs.parts[index] : 0
            if left != right { return left < right }
        }
        return false
    }

    /// 默认合成的 `==` 会把 `[0, 2]` 和 `[0, 2, 0]` 判成不等，
    /// 和上面 `<` 的补 0 语义打架 —— `isAvailable` 用的是 `<`，但测试和
    /// `XCTAssertEqual` 走的是 `==`。两边对齐，都按补 0 比。
    static func == (lhs: Self, rhs: Self) -> Bool {
        !(lhs < rhs) && !(rhs < lhs)
    }

    var description: String {
        parts.map(String.init).joined(separator: ".")
    }
}
