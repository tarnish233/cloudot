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

    private func apply() {
        Task {
            await model.apply()
        }
    }

    private func refresh() {
        Task {
            // 用户主动点的，菜单栏机器人才扫眼 —— 动效要对应得上操作
            await model.refresh(userInitiated: true)
        }
    }

    private func quit() {
        NSApp.terminate(nil)
    }
}
