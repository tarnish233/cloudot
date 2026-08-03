import SwiftUI

struct MenuBarContent: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            Group {
                if let error = model.locateError {
                    MenuBarMissingBinaryView(error: error)
                } else if model.needsSetup {
                    SetupCard(model: model, compact: true)
                } else if let status = model.status {
                    MenuBarStatusContent(status: status, model: model)
                } else {
                    ProgressView("读取状态中…")
                        .frame(maxWidth: .infinity, minHeight: 104)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(.horizontal, CloudotTheme.sectionSpacing)
            .padding(.vertical, CloudotTheme.compactSpacing)
        }
        .scrollIndicators(.automatic)
        .frame(maxHeight: CloudotTheme.menuContentMaximumHeight)
    }
}
