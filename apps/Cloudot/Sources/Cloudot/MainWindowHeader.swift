import SwiftUI

struct MainWindowHeader: View {
    let pane: Pane
    let model: AppModel

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .center, spacing: CloudotTheme.sectionSpacing) {
                title
                Spacer(minLength: CloudotTheme.sectionSpacing)
                actions
            }

            VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                title
                actions
            }
        }
        .padding(.horizontal, CloudotTheme.pagePadding)
        .padding(.vertical, CloudotTheme.sectionSpacing)
    }

    private var title: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(pane.title)
                .font(.title2)
                .bold()
            Text(pane.subtitle)
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }

    private var actions: some View {
        HStack(spacing: CloudotTheme.compactSpacing) {
            if model.isBusy {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel(model.isWorking ? "正在执行操作" : "正在刷新状态")
            }

            Button("刷新", systemImage: "arrow.clockwise", action: refresh)
                .keyboardShortcut("r", modifiers: [.command])
                .disabled(model.isBusy)

            Button("同步", systemImage: "arrow.triangle.2.circlepath", action: sync)
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("s", modifiers: [.command])
                .disabled(!model.isReady || model.isBusy)
        }
    }

    private func refresh() {
        Task {
            // 用户主动点的，菜单栏机器人才扫眼 —— 动效要对应得上操作
            await model.refresh(userInitiated: true)
        }
    }

    private func sync() {
        Task {
            await model.sync()
        }
    }
}
