import SwiftUI

struct MenuBarFooter: View {
    let model: AppModel
    let openMainWindow: () -> Void

    /// 底行「检查更新」的一次性反馈（已是最新 / 失败），几秒后清掉。
    /// 不用 model.banner：Setup 态没有 BannerView，主窗口又可能不在前台。
    @State private var updateHint: String?

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
                .disabled(!model.canSync)
                .help(model.needsSetup
                      ? "先完成初始化再同步"
                      : "提交本地改动、拉取远端并推送（⌘S）")
            }

            HStack(spacing: CloudotTheme.compactSpacing) {
                if let updateHint {
                    Text(updateHint)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else if let lastRefresh = model.lastRefresh {
                    Label(Format.relative(lastRefresh), systemImage: "clock")
                        .font(.callout)
                        .foregroundStyle(.tertiary)
                        .accessibilityLabel("上次更新：\(Format.relative(lastRefresh))")
                }

                Spacer(minLength: CloudotTheme.compactSpacing)

                // 文字按钮放底行，不挤上面「设置 / 刷新 / 同步」三联。
                // 有新版本时上面已经出现「更新到 X」大按钮；这里始终保留入口，
                // 方便主动查一次（force，清掉缓存）。
                Button(checkUpdateLabel, action: checkForUpdate)
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .disabled(model.isCheckingForUpdate || model.isBusy)
                    .help("检查是否有新版本")

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

    private var checkUpdateLabel: String {
        if model.isCheckingForUpdate { return "检查中…" }
        return "检查更新"
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

    private func checkForUpdate() {
        updateHint = nil
        Task {
            await model.checkForUpdate(force: true)
            if let check = model.updateCheck {
                if check.isAvailable {
                    // 上面会自动出现「更新到 X」大按钮
                    updateHint = nil
                } else {
                    updateHint = "已是最新 \(check.current.description)"
                    clearHintSoon()
                }
            } else {
                // force 失败时 AppModel 会写 failure banner；底行再给一句短的
                updateHint = model.banner?.title ?? "检查失败"
                clearHintSoon()
            }
        }
    }

    private func clearHintSoon() {
        Task {
            try? await Task.sleep(for: .seconds(3))
            updateHint = nil
        }
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
