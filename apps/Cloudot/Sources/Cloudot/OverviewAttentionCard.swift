import SwiftUI

struct OverviewAttentionCard: View {
    let orphans: [Orphan]

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
            Label("需要处理", systemImage: "exclamationmark.triangle.fill")
                .font(.headline)
                .foregroundStyle(.orange)

            GroupBox {
                VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                    ForEach(orphans) { orphan in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(orphan.target)
                                .font(.system(.callout, design: .monospaced))
                                .textSelection(.enabled)
                            Text(orphan.kind.label)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }

                        if orphan.id != orphans.last?.id {
                            Divider()
                        }
                    }

                    Text("“落地到本机”会从 Git 历史或备份取回内容，并还原为实体文件。")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(CloudotTheme.compactSpacing)
            }
        }
    }
}
