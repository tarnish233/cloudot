import SwiftUI

/// 拉取冲突后的选边面板：文件列表 + monospaced diff + 用远端 / 留本地。
struct ConflictSheet: View {
    let model: AppModel
    let report: ConflictReport

    @State private var selectedPath: String?

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            content
            Divider()
            footer
        }
        .frame(minWidth: 640, minHeight: 420)
        .onAppear {
            if selectedPath == nil {
                selectedPath = report.files.first?.path
            }
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 4) {
                Text("远端有冲突改动")
                    .font(.headline)
                Text("已自动回滚，本地配置没被改坏。查看 diff 后选一边。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Text("\(report.branch)  ↔  \(report.remoteRef)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            Button("稍后") { model.dismissConflict() }
                .keyboardShortcut(.cancelAction)
        }
        .padding(CloudotTheme.sectionSpacing)
    }

    private var content: some View {
        HSplitView {
            List(report.files, selection: $selectedPath) { file in
                Text(file.path)
                    .font(.callout.monospaced())
                    .lineLimit(2)
                    .truncationMode(.middle)
                    .tag(file.path)
            }
            .frame(minWidth: 180, idealWidth: 220)

            ScrollView {
                if let file = report.files.first(where: { $0.path == selectedPath }) {
                    VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                        if file.truncated {
                            Text("diff 过长，已截断")
                                .font(.caption)
                                .foregroundStyle(.orange)
                        }
                        Text(file.diff.isEmpty ? "（无文本差异，可能是二进制或新建/删除）" : file.diff)
                            .font(.system(.caption, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(CloudotTheme.sectionSpacing)
                } else {
                    ContentUnavailableView("选择一个文件", systemImage: "doc.text.magnifyingglass")
                        .frame(maxWidth: .infinity, minHeight: 200)
                }
            }
            .frame(minWidth: 360)
        }
    }

    private var footer: some View {
        HStack(spacing: CloudotTheme.compactSpacing) {
            VStack(alignment: .leading, spacing: 2) {
                Text("用远端：丢弃本地未推送的提交，对齐 \(report.remoteRef)。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text("留本地：把本地强推上去（--force-with-lease，不会盖掉你没见过的远端提交）。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: CloudotTheme.sectionSpacing)
            Button("用远端覆盖本地") {
                Task { await model.resolveConflict(.theirs) }
            }
            .disabled(model.isBusy)
            Button("保留本地并推送") {
                Task { await model.resolveConflict(.ours) }
            }
            .buttonStyle(.borderedProminent)
            .disabled(model.isBusy)
        }
        .padding(CloudotTheme.sectionSpacing)
    }
}
