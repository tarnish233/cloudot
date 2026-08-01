import SwiftUI

struct DoctorPane: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            Group {
                if let report = model.doctor {
                    LazyVStack(alignment: .leading, spacing: CloudotTheme.sectionSpacing) {
                        Label(
                            report.ok ? "没有致命问题" : "存在需要处理的问题",
                            systemImage: report.ok ? "checkmark.seal.fill" : "xmark.seal.fill"
                        )
                        .foregroundStyle(report.ok ? SystemTheme.accentColor : .red)
                        .font(.headline)

                        ForEach(report.checks.sorted(by: { $0.level > $1.level })) { check in
                            DoctorCheckRow(check: check)
                        }
                    }
                } else {
                    ProgressView("体检中…")
                        .frame(maxWidth: .infinity, minHeight: 220)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(CloudotTheme.pagePadding)
        }
        .scrollContentBackground(.visible)
    }
}
