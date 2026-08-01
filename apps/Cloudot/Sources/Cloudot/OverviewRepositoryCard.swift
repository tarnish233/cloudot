import SwiftUI

struct OverviewRepositoryCard: View {
    let git: GitInfo
    let root: String

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
            Label("仓库", systemImage: "externaldrive")
                .font(.headline)

            GroupBox {
                VStack(spacing: CloudotTheme.cardPadding) {
                    LabeledContent("远端") {
                        Text(remote)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                    }

                    Divider()

                    LabeledContent("本地目录") {
                        Text(root)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                    }
                }
                .padding(CloudotTheme.compactSpacing)
            }
        }
    }

    private var remote: String {
        guard git.repo else { return "尚未初始化 Git 仓库" }
        return git.remote ?? "未配置"
    }
}
