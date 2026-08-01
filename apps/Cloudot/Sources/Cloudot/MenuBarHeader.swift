import SwiftUI

struct MenuBarHeader: View {
    let model: AppModel

    var body: some View {
        HStack(spacing: CloudotTheme.sectionSpacing) {
            ZStack {
                Circle()
                    .fill(model.overallLevel.tint.opacity(0.14))
                Image(systemName: model.overallLevel.symbol)
                    .font(.title3)
                    .foregroundStyle(model.overallLevel.tint)
                    .symbolRenderingMode(.hierarchical)
            }
            .frame(width: 36, height: 36)
            .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 2) {
                Text(model.headline)
                    .font(.headline)
                if let status = model.status {
                    Text(gitSummary(status.git))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }

            Spacer(minLength: CloudotTheme.compactSpacing)

            if model.isBusy {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel(model.isWorking ? "正在执行操作" : "正在刷新状态")
            }
        }
        .padding(.horizontal, CloudotTheme.sectionSpacing)
        .padding(.top, CloudotTheme.sectionSpacing)
        .padding(.bottom, CloudotTheme.compactSpacing)
        .accessibilityElement(children: .combine)
    }

    private func gitSummary(_ git: GitInfo) -> String {
        guard git.repo else { return "store 还不是 git 仓库" }
        var parts: [String] = []
        if let branch = git.branch { parts.append(branch) }
        if let ahead = git.ahead, ahead > 0 { parts.append("领先 \(ahead)") }
        if let behind = git.behind, behind > 0 { parts.append("落后 \(behind)") }
        if !git.dirty.isEmpty { parts.append("\(git.dirty.count) 处未提交") }
        return parts.isEmpty ? "仓库状态正常" : parts.joined(separator: " · ")
    }
}
