import SwiftUI

struct MainWindow: View {
    let model: AppModel
    @State private var pane: Pane = .overview
    @State private var columnVisibility: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            MainWindowSidebar(selection: $pane)
        } detail: {
            VStack(spacing: 0) {
                MainWindowHeader(pane: pane, model: model)

                Divider()

                if let banner = model.banner {
                    BannerView(banner: banner) {
                        model.banner = nil
                    }
                    .padding(.horizontal, CloudotTheme.pagePadding)
                    .padding(.top, CloudotTheme.sectionSpacing)
                }

                switch pane {
                case .overview:
                    OverviewPane(model: model)
                case .apps:
                    AppsPane(model: model)
                case .doctor:
                    DoctorPane(model: model)
                case .backups:
                    BackupsPane(model: model)
                case .about:
                    AboutPane(model: model)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(.background)
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar(removing: .sidebarToggle)
        .tint(SystemTheme.accentColor)
        .frame(
            minWidth: CloudotTheme.windowMinimumWidth,
            minHeight: CloudotTheme.windowMinimumHeight
        )
        .confirm(model)
        .task {
            // startAutoRefresh 内部第一件事就是 refresh()，所以首次打开窗口
            // 自然会拉一次完整数据，不用再单独补一刀。
            model.startAutoRefresh()
        }
    }
}
