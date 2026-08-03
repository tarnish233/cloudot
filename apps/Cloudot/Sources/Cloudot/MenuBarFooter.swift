import SwiftUI

struct MenuBarFooter: View {
    let model: AppModel
    let openMainWindow: () -> Void

    var body: some View {
        VStack(spacing: CloudotTheme.compactSpacing) {
            Divider()

            if model.hasApplyableWork {
                Button(action: apply) {
                    Label("修复并落地", systemImage: "arrow.down.circle")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
                .help("从 Git 历史或备份恢复配置并重新落地")
                .keyboardShortcut(.return, modifiers: [.command, .shift])
                .disabled(model.isBusy)
            }

            // 更新相关的行只在真有事时出现，和上面「修复并落地」一个路子 ——
            // 常驻一个多数时候没用的按钮会挤掉三联那排的标签（面板只有 340pt 宽）。
            if let version = model.pendingRestartVersion {
                Button(action: model.restartForUpdate) {
                    Label("重启以启用 \(version)", systemImage: "arrow.clockwise.circle")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .help("更新已装好，重启应用后生效")
            } else if let check = model.updateCheck, check.isAvailable {
                // 用 `.description`：SemanticVersion 直接插进 LocalizedStringKey 会走
                // debug description 那条弃用路径，文案里也可能不是用户看到的版本号。
                let latest = check.latest.description
                Button(action: startUpdate) {
                    Label("更新到 \(latest)", systemImage: "arrow.down.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
                .help("下载并安装 \(latest)，装完需要重启应用")
                .disabled(model.isBusy)
            }

            HStack(spacing: CloudotTheme.compactSpacing) {
                Button(action: openMainWindow) {
                    Label("设置", systemImage: "macwindow")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.regular)
                .accessibilityLabel("Cloudot 设置")
                .help("打开 Cloudot 设置")

                Button(action: refresh) {
                    Label("刷新", systemImage: "arrow.clockwise")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.regular)
                .keyboardShortcut("r", modifiers: [.command])
                .disabled(model.isBusy)
                .help(refreshHelp)

                Button(action: sync) {
                    Label("立即同步", systemImage: "arrow.triangle.2.circlepath")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.regular)
                .keyboardShortcut("s", modifiers: [.command])
                .disabled(!model.isReady || model.isBusy)
                .help("提交本地改动、拉取远端并推送（⌘S）")
            }

            HStack(spacing: CloudotTheme.compactSpacing) {
                if let lastRefresh = model.lastRefresh {
                    Label(Format.relative(lastRefresh), systemImage: "clock")
                        .font(.callout)
                        .foregroundStyle(.tertiary)
                        .accessibilityLabel("上次更新：\(Format.relative(lastRefresh))")
                }

                Spacer(minLength: CloudotTheme.compactSpacing)

                Button("退出", systemImage: "power", action: quit)
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .keyboardShortcut("q", modifiers: [.command])
                    .help("退出 Cloudot（⌘Q）")
            }
        }
        .padding(.horizontal, CloudotTheme.sectionSpacing)
        .padding(.top, CloudotTheme.compactSpacing)
        .padding(.bottom, CloudotTheme.sectionSpacing)
    }

    private var refreshHelp: String {
        guard let lastRefresh = model.lastRefresh else {
            return "刷新状态（⌘R）"
        }
        return "刷新状态 · 上次更新：\(Format.relative(lastRefresh))（⌘R）"
    }

    private func sync() {
        Task {
            await model.sync()
        }
    }

    private func startUpdate() {
        guard let check = model.updateCheck else { return }
        model.pending = .installUpdate(
            from: check.current.description,
            to: check.latest.description
        )
    }

    private func apply() {
        Task {
            await model.apply()
        }
    }

    private func refresh() {
        Task {
            // 用户主动点的，菜单栏图标才变 —— 反馈要对应得上操作
            await model.refresh(userInitiated: true)
        }
    }

    private func quit() {
        NSApp.terminate(nil)
    }
}
