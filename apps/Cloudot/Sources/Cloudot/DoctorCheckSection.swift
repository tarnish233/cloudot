import SwiftUI

struct DoctorCheckSection: View {
    let category: DoctorCategory
    let checks: [Check]

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
            SectionHeader(title: category.title, symbol: category.symbol, count: checks.count)

            GroupBox {
                VStack(spacing: 0) {
                    ForEach(checks) { check in
                        DoctorCheckRow(check: check)

                        if check.id != checks.last?.id {
                            Divider()
                                .padding(.leading, 42)
                        }
                    }
                }
            }
        }
    }
}
