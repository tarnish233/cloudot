import SwiftUI

struct OverviewPane: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            Group {
                if let error = model.locateError {
                    ContentUnavailableView {
                        Label(error.summary, systemImage: "xmark.octagon.fill")
                    } description: {
                        Text(error.errorDescription ?? "无法读取 cloudot 状态。")
                            .textSelection(.enabled)
                    }
                } else if let status = model.status {
                    statusContent(status)
                } else {
                    ProgressView("读取状态中…")
                        .frame(maxWidth: .infinity, minHeight: 220)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(CloudotTheme.pagePadding)
        }
        .scrollContentBackground(.visible)
    }

    private func statusContent(_ status: Status) -> some View {
        VStack(alignment: .leading, spacing: CloudotTheme.sectionSpacing) {
            HStack(alignment: .center, spacing: CloudotTheme.cardPadding) {
                Image(systemName: model.overallLevel.symbol)
                    .font(.largeTitle)
                    .foregroundStyle(model.overallLevel.tint)
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 2) {
                    Text(model.headline)
                        .font(.title2)
                        .bold()
                    Text("设备 \(status.device)")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }

                Spacer(minLength: CloudotTheme.compactSpacing)

                if model.hasApplyableWork {
                    Button("落地到本机", systemImage: "arrow.down.circle", action: apply)
                        .disabled(model.isBusy)
                }
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel("\(model.headline)，设备 \(status.device)")

            GroupBox("仓库与配置") {
                VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                    ForEach(rows(status)) { row in
                        LabeledContent(row.label) {
                            Text(row.value)
                                .multilineTextAlignment(.trailing)
                                .textSelection(.enabled)
                        }
                        if row.id != "目录" {
                            Divider()
                        }
                    }
                }
                .padding(CloudotTheme.compactSpacing)
            }

            if !status.orphans.isEmpty {
                GroupBox {
                    VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                        Label("需要处理", systemImage: "exclamationmark.triangle.fill")
                            .font(.headline)
                            .foregroundStyle(.orange)

                        ForEach(status.orphans) { orphan in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(orphan.target)
                                    .font(.system(.callout, design: .monospaced))
                                    .textSelection(.enabled)
                                Text(orphan.kind.label)
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                            }
                        }

                        Text("“落地到本机”会从 git 历史或备份取回内容，并还原为实体文件。")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(CloudotTheme.compactSpacing)
                }
            }
        }
    }

    private func rows(_ status: Status) -> [OverviewRow] {
        [
            OverviewRow(
                label: "分支",
                value: status.git.repo
                    ? "\(status.git.branch ?? "无分支") @ \(status.git.head ?? "无提交")"
                    : "store 还不是 git 仓库"
            ),
            OverviewRow(label: "远端", value: status.git.remote ?? "未配置"),
            OverviewRow(label: "与远端", value: syncState(status.git)),
            OverviewRow(label: "纳管", value: status.apps.isEmpty ? "无" : "\(status.apps.count) 个应用"),
            OverviewRow(label: "目录", value: status.root),
        ]
    }

    private func syncState(_ git: GitInfo) -> String {
        guard git.repo else { return "—" }
        var parts: [String] = []
        if let ahead = git.ahead, ahead > 0 { parts.append("领先 \(ahead)") }
        if let behind = git.behind, behind > 0 { parts.append("落后 \(behind)") }
        if !git.dirty.isEmpty { parts.append("\(git.dirty.count) 处未提交") }
        return parts.isEmpty ? "一致" : parts.joined(separator: " · ")
    }

    private func apply() {
        Task {
            await model.apply()
        }
    }
}

private struct OverviewRow: Identifiable {
    let label: String
    let value: String

    var id: String { label }
}
