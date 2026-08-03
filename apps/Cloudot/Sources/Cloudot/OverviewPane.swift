import SwiftUI

struct OverviewPane: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            Group {
                if let error = model.locateError {
                    ContentUnavailableView {
                        Label(error.summary, systemImage: "xmark.octagon.fill")
                    } description: {
                        Text(error.errorDescription ?? "无法读取 Cloudot 状态。")
                            .textSelection(.enabled)
                    }
                } else if model.needsSetup {
                    SetupCard(model: model)
                        .frame(maxWidth: 520, alignment: .leading)
                } else if let status = model.status {
                    OverviewContent(status: status, model: model)
                } else {
                    ProgressView("读取状态中…")
                        .frame(maxWidth: .infinity, minHeight: 220)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(CloudotTheme.pagePadding)
        }
        .scrollContentBackground(.visible)
    }
}
