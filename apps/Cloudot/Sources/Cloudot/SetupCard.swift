import SwiftUI

/// 未初始化时的引导：可选 remote + 一键 init。
///
/// 菜单栏和主窗口共用；`compact` 时收紧间距，适配 340pt 宽的面板。
struct SetupCard: View {
    let model: AppModel
    var compact: Bool = false

    @State private var remote: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? CloudotTheme.compactSpacing : CloudotTheme.sectionSpacing) {
            Label("开始设置", systemImage: "arrow.triangle.2.circlepath.circle.fill")
                .font(compact ? .headline : .title3.bold())
                .foregroundStyle(SystemTheme.accentColor)

            Text(compact
                 ? "把本机连上你的 dotfiles 仓库，或先在本地初始化。"
                 : "Cloudot 通过 git 在多台 Mac 之间同步配置。填一个已有的仓库地址，或留空只在本地初始化。")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            TextField("git@github.com:you/dotfiles.git（可选）", text: $remote)
                .textFieldStyle(.roundedBorder)
                .font(.body.monospaced())
                .disabled(model.isBusy)

            Button(action: initialize) {
                Label("初始化", systemImage: "checkmark.circle")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(compact ? .regular : .large)
            .disabled(!model.isReady || model.isBusy)
            .keyboardShortcut(.return, modifiers: [.command])
        }
        .padding(compact ? CloudotTheme.cardPadding : CloudotTheme.sectionSpacing)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary, in: .rect(cornerRadius: 12))
    }

    private func initialize() {
        let trimmed = remote.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            await model.initialize(remote: trimmed.isEmpty ? nil : trimmed)
        }
    }
}
