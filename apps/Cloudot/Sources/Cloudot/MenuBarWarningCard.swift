import SwiftUI

struct MenuBarWarningCard: View {
    let orphanCount: Int

    var body: some View {
        HStack(alignment: .top, spacing: CloudotTheme.compactSpacing) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("有 \(orphanCount) 个配置读不到了")
                    .font(.headline)
                Text("点“落地”会从 git 历史或备份取回内容。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(CloudotTheme.cardPadding)
        .background(.orange.opacity(0.10), in: .rect(cornerRadius: 12))
        .accessibilityElement(children: .combine)
    }
}
