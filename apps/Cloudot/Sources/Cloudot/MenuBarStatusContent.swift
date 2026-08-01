import SwiftUI

struct MenuBarStatusContent: View {
    let status: Status
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
            if !status.orphans.isEmpty {
                MenuBarWarningCard(orphanCount: status.orphans.count)
            }

            if status.apps.isEmpty {
                ContentUnavailableView(
                    "还没有同步应用",
                    systemImage: "app.badge",
                    description: Text("打开设置查看可同步的应用。")
                )
                .frame(minHeight: 128)
                .background(.quaternary, in: .rect(cornerRadius: 12))
            } else {
                LazyVStack(spacing: 0) {
                    ForEach(status.apps) { app in
                        MenuBarApplicationRow(app: app)

                        if app.id != status.apps.last?.id {
                            Divider()
                                .padding(.leading, 42)
                        }
                    }
                }
                .padding(.vertical, 2)
                .background(.quaternary, in: .rect(cornerRadius: 12))
            }

            if let banner = model.banner {
                BannerView(banner: banner, compact: true) {
                    model.banner = nil
                }
                .padding(CloudotTheme.cardPadding)
                .background(.quaternary, in: .rect(cornerRadius: 12))
            }
        }
    }
}
