import SwiftUI

struct DoctorPane: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            Group {
                if let report = model.doctor {
                    LazyVStack(alignment: .leading, spacing: CloudotTheme.sectionSpacing) {
                        VStack(alignment: .leading, spacing: 4) {
                            Label(
                                report.ok ? "整体状态良好" : "发现需要处理的问题",
                                systemImage: report.ok ? "checkmark.seal.fill" : "xmark.seal.fill"
                            )
                            .foregroundStyle(report.ok ? SystemTheme.accentColor : .red)
                            .font(.title3.bold())

                            Text("已完成 \(report.checks.count) 项检查，并按用途归类。")
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                        .accessibilityElement(children: .combine)

                        ForEach(DoctorCategory.allCases) { category in
                            let checks = category.checks(in: report)
                            if !checks.isEmpty {
                                DoctorCheckSection(category: category, checks: checks)
                            }
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
