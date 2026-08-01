import SwiftUI

struct MenuBarFooter: View {
    let model: AppModel
    let openMainWindow: () -> Void

    var body: some View {
        HStack(spacing: CloudotTheme.compactSpacing) {
            Button("主窗口", systemImage: "macwindow", action: openMainWindow)
                .buttonStyle(.borderless)

            Spacer(minLength: CloudotTheme.compactSpacing)

            if model.hasApplyableWork {
                Button("落地", systemImage: "arrow.down.circle", action: apply)
                    .disabled(model.isBusy)
            }

            Button("刷新状态", systemImage: "arrow.clockwise", action: refresh)
                .labelStyle(.iconOnly)
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("r", modifiers: [.command])
                .disabled(model.isBusy)
                .help(refreshHelp)

            Button("立即同步", systemImage: "arrow.triangle.2.circlepath", action: sync)
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("s", modifiers: [.command])
                .disabled(!model.isReady || model.isBusy)

            Button("退出 Cloudot", systemImage: "power", action: quit)
                .labelStyle(.iconOnly)
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("q", modifiers: [.command])
                .help("退出 Cloudot（⌘Q）")
        }
        .padding(.horizontal, CloudotTheme.sectionSpacing)
        .padding(.top, 4)
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
