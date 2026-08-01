import SwiftUI

struct OverviewContent: View {
    let status: Status
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.sectionSpacing) {
            OverviewStatusCard(
                headline: model.headline,
                level: model.overallLevel,
                device: status.device,
                showsApplyAction: model.hasApplyableWork,
                isBusy: model.isBusy,
                onApply: apply
            )

            Label("同步概况", systemImage: "chart.bar")
                .font(.headline)

            metrics

            OverviewRepositoryCard(git: status.git, root: status.root)

            if !status.orphans.isEmpty {
                OverviewAttentionCard(orphans: status.orphans)
            }
        }
    }

    private var metrics: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: CloudotTheme.compactSpacing) {
                metricCards
            }

            VStack(spacing: CloudotTheme.compactSpacing) {
                metricCards
            }
        }
    }

    @ViewBuilder
    private var metricCards: some View {
        OverviewMetricCard(
            title: "同步应用",
            value: status.apps.count.formatted(),
            symbol: "app.badge.checkmark"
        )
        OverviewMetricCard(
            title: "配置文件",
            value: fileCount.formatted(),
            symbol: "doc.on.doc"
        )
        OverviewMetricCard(
            title: "当前分支",
            value: branchName,
            symbol: "arrow.triangle.branch"
        )
    }

    private var fileCount: Int {
        status.apps.reduce(0) { $0 + $1.files.count }
    }

    private var branchName: String {
        guard status.git.repo else { return "未初始化" }
        return status.git.branch ?? "无分支"
    }

    private func apply() {
        Task {
            await model.apply()
        }
    }
}
