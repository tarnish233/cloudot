import SwiftUI

struct MainWindowSidebar: View {
    @Binding var selection: Pane

    var body: some View {
        List(Pane.allCases, selection: $selection) { item in
            Label(item.title, systemImage: item.symbol)
                .tag(item)
                .accessibilityHint(item.accessibilityHint)
        }
        .listStyle(.sidebar)
        .scrollContentBackground(.hidden)
        .navigationSplitViewColumnWidth(
            min: CloudotTheme.sidebarMinimumWidth,
            ideal: CloudotTheme.sidebarIdealWidth,
            max: CloudotTheme.sidebarMaximumWidth
        )
        .background(.regularMaterial)
    }
}
