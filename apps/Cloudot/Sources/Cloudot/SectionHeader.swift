import SwiftUI

/// 页面内的分节标题：图标 + 标题 +（可选）计数。
///
/// 抽出来是因为原先三处各写各的 —— 体检页带图标、应用页只有文字、概览页直接内联
/// `Label`，同一个窗口里出现了三种分节样式。计数用次要色，避免和标题抢重量。
struct SectionHeader: View {
    let title: String
    let symbol: String
    var count: Int?

    var body: some View {
        HStack(spacing: 6) {
            Label(title, systemImage: symbol)
                .font(.headline)

            if let count {
                Text(count, format: .number)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
        .accessibilityElement(children: .combine)
    }
}
