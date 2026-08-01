import SwiftUI

struct DoctorCheckSection: View {
    let category: DoctorCategory
    let checks: [Check]

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
            HStack(spacing: 6) {
                Label(category.title, systemImage: category.symbol)
                    .font(.headline)

                Text(checks.count, format: .number)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)

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
