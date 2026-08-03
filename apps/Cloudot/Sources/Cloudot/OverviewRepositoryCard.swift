import SwiftUI

struct OverviewRepositoryCard: View {
    let git: GitInfo
    let root: String

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
            SectionHeader(title: "仓库", symbol: "externaldrive")

            GroupBox {
                // `LabeledContent` 在 GroupBox 里有两处要显式给约束，否则不对：
                //   1. 整体不撑满宽度的话它按内容收缩并居中，标签和值挤在中间；
                //   2. 值那一侧不撑满并右对齐的话，会紧跟着标签排在左边。
                // macOS 的规矩是标签贴左、值贴右，两者之间留白。
                VStack(alignment: .leading, spacing: CloudotTheme.cardPadding) {
                    LabeledContent("远端") {
                        value(remote)
                    }

                    Divider()

                    LabeledContent("本地目录") {
                        value(root)
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(CloudotTheme.compactSpacing)
            }
        }
    }

    /// 路径可能很长，中间截断比尾部截断好 —— 结尾那几段才是区分度最高的。
    private func value(_ text: String) -> some View {
        Text(text)
            .lineLimit(1)
            .truncationMode(.middle)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private var remote: String {
        guard git.repo else { return "尚未初始化 Git 仓库" }
        return git.remote ?? "未配置"
    }
}
