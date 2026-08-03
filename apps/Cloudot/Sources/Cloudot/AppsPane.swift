import SwiftUI

struct AppsPane: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: CloudotTheme.sectionSpacing) {
                if let managedApps = model.status?.apps, !managedApps.isEmpty {
                    SectionHeader(title: "已同步", symbol: "checkmark.circle", count: managedApps.count)

                    ForEach(managedApps) { app in
                        ManagedAppCard(app: app, model: model)
                    }
                }

                if !model.detectedCandidates.isEmpty {
                    SectionHeader(title: "可同步", symbol: "app.dashed", count: model.detectedCandidates.count)

                    GroupBox {
                        VStack(spacing: 0) {
                            ForEach(model.detectedCandidates) { app in
                                HStack(spacing: CloudotTheme.compactSpacing) {
                                    Label(app.name, systemImage: "app.dashed")
                                    Spacer()
                                    Button("同步") {
                                        // 先拉一次 show 拿到真实路径，确认框才能列出会动哪些文件
                                        Task {
                                            await model.requestAdopt(id: app.id, name: app.name)
                                        }
                                    }
                                    .disabled(model.isBusy)
                                }
                                .padding(.vertical, CloudotTheme.compactSpacing)

                                if app.id != model.detectedCandidates.last?.id {
                                    Divider()
                                }
                            }
                        }
                        .padding(.horizontal, CloudotTheme.compactSpacing)
                    }
                }

                if model.status?.apps.isEmpty != false && model.detectedCandidates.isEmpty {
                    ContentUnavailableView(
                        "没有可管理的应用",
                        systemImage: "app.badge",
                        description: Text("Cloudot 暂未发现已同步或可同步的应用配置。")
                    )
                    .frame(maxWidth: .infinity, minHeight: 220)
                }

                if !model.unavailableApps.isEmpty {
                    Label(
                        "本机未检测到：\(model.unavailableApps.map(\.name).joined(separator: "、"))",
                        systemImage: "info.circle"
                    )
                    .font(.callout)
                    .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(CloudotTheme.pagePadding)
        }
        .scrollContentBackground(.visible)
    }
}
