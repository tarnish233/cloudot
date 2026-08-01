import SwiftUI

struct OverviewMetricCard: View {
    let title: String
    let value: String
    let symbol: String

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 6) {
                Label(title, systemImage: symbol)
                    .font(.callout)
                    .foregroundStyle(.secondary)

                Text(value)
                    .font(.title3.bold())
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
            .frame(maxWidth: .infinity, minHeight: 58, alignment: .leading)
            .padding(CloudotTheme.compactSpacing)
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .combine)
    }
}
