import AppKit
import SwiftUI

struct AboutPane: View {
    let model: AppModel

    var body: some View {
        Form {
            Section {
                identity
                    .listRowInsets(EdgeInsets(top: CloudotTheme.sectionSpacing, leading: 0,
                                              bottom: CloudotTheme.sectionSpacing, trailing: 0))
            }

            if let version = model.pendingRestartVersion {
                Section {
                    LabeledContent("\(version) 已就绪") {
                        Button("重启应用") { model.restartForUpdate() }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                    }
                }
            }

            Section("版本") {
                // 正常路径只显示一行 App 版本。CLI 与 App 一致时不必分行；
                // 只有两边分叉时才标橙提醒（否则界面只会报「输出异常」）。
                LabeledContent("版本", value: Self.appVersion)

                if let cliVersion = model.cliVersion, cliVersion != Self.appVersion {
                    LabeledContent("命令行工具") {
                        VStack(alignment: .trailing, spacing: 2) {
                            Text(cliVersion).foregroundStyle(Color.orange)
                            if let path = model.cliPath {
                                Text(path)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                                    .textSelection(.enabled)
                            }
                        }
                    }
                } else if model.cliVersion == nil && !model.isReady {
                    LabeledContent("命令行工具", value: "未找到")
                }

                LabeledContent("最新版本") { latest }
            }

            Section("位置") {
                if let path = model.cliPath {
                    pathRow("命令行工具", path)
                }
                if let root = model.status?.root {
                    pathRow("配置仓库", root)
                }
            }

            Section("项目") {
                Link(destination: Self.repositoryURL) {
                    LabeledContent("源码") {
                        Label(Self.repositoryURL.host() ?? "GitHub", systemImage: "arrow.up.right")
                            .labelStyle(.titleAndIcon)
                    }
                }
                .buttonStyle(.plain)
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
        .task {
            await model.loadCLIVersion()
            await model.checkForUpdate()
        }
    }

    // MARK: - 组成部分

    private var identity: some View {
        HStack(spacing: CloudotTheme.sectionSpacing) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 64, height: 64)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 4) {
                Text("Cloudot")
                    .font(.title2.bold())
                Text("用 git 在多台 Mac 之间同步 dotfiles")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    /// 路径行。路径长且尾部区分度最高，所以中间截断并允许选中复制 ——
    /// 出问题时用户多半要把它贴到别处。
    private func pathRow(_ label: String, _ value: String) -> some View {
        LabeledContent(label) {
            Text(value)
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }

    /// 最新版本那一格：查到了显示版本号 + 更新按钮；始终提供「检查更新」。
    @ViewBuilder
    private var latest: some View {
        HStack(spacing: CloudotTheme.compactSpacing) {
            if let check = model.updateCheck {
                if check.isAvailable {
                    Text(check.latest.description).foregroundStyle(Color.orange)
                    Button("更新") {
                        model.pending = .installUpdate(
                            from: check.current.description,
                            to: check.latest.description
                        )
                    }
                    .controlSize(.small)
                    .disabled(model.isBusy || model.pendingRestartVersion != nil)
                } else {
                    Text("已是最新").foregroundStyle(.secondary)
                }
            } else if model.isCheckingForUpdate {
                Text("检查中…").foregroundStyle(.secondary)
            } else {
                Text("尚未检查").foregroundStyle(.secondary)
            }

            Button("检查更新") {
                Task { await model.checkForUpdate(force: true) }
            }
            .controlSize(.small)
            .disabled(model.isCheckingForUpdate || model.isBusy)
        }
    }

    // MARK: - 常量

    /// 版本号在 `AppModel` 上，不在这里另存一份 —— 更新逻辑也要用它。
    private static var appVersion: String { AppModel.appVersion }

    private static let repositoryURL = URL(string: "https://github.com/tarnish233/cloudot")!
}
