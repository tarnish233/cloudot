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
                LabeledContent("应用", value: Self.appVersion)
                // CLI 版本单独显示，不是冗余：GUI 调的可能是 bundle 内自带的那份，
                // 也可能是 ~/.cargo/bin 里的旧二进制。两个版本对不上时界面只会报
                // 「输出异常」，摆在这里能一眼看出来。
                LabeledContent("命令行工具") {
                    if let version = model.cliVersion {
                        // 版本对不上时标黄。三元两边都得是 Color ——
                        // `.primary` 是 HierarchicalShapeStyle，和 `.orange` 类型不通。
                        Text(version)
                            .foregroundStyle(version == Self.appVersion ? Color.primary : Color.orange)
                    } else if model.isReady {
                        Text("读取中…").foregroundStyle(.secondary)
                    } else {
                        Text("未找到").foregroundStyle(.secondary)
                    }
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

    /// 最新版本那一格：查到了显示版本号 + 更新按钮，没查到就说明状态。
    @ViewBuilder
    private var latest: some View {
        if let check = model.updateCheck {
            if check.isAvailable {
                HStack(spacing: CloudotTheme.compactSpacing) {
                    // 有新版时标黄，和上面「CLI 版本对不上」用的是同一套视觉语言
                    Text(check.latest.description).foregroundStyle(Color.orange)
                    Button("更新") {
                        model.pending = .installUpdate(
                            from: check.current.description,
                            to: check.latest.description
                        )
                    }
                    .controlSize(.small)
                    .disabled(model.isBusy || model.pendingRestartVersion != nil)
                }
            } else {
                Text("已是最新").foregroundStyle(.secondary)
            }
        } else if model.isCheckingForUpdate {
            Text("检查中…").foregroundStyle(.secondary)
        } else {
            Text("查不到").foregroundStyle(.secondary)
        }
    }

    // MARK: - 常量

    /// 版本号在 `AppModel` 上，不在这里另存一份 —— 更新逻辑也要用它。
    private static var appVersion: String { AppModel.appVersion }

    private static let repositoryURL = URL(string: "https://github.com/tarnish233/cloudot")!
}
