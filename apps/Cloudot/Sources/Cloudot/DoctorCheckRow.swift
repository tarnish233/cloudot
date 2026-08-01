import SwiftUI

struct DoctorCheckRow: View {
    let check: Check

    var body: some View {
        GroupBox {
            HStack(alignment: .top, spacing: CloudotTheme.cardPadding) {
                Image(systemName: check.level.symbol)
                    .foregroundStyle(check.level.tint)
                    .font(.title3)
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 4) {
                    Text(check.name)
                        .font(.system(.callout, design: .monospaced))
                        .bold()
                    Text(check.message)
                        .textSelection(.enabled)
                    if let hint = check.hint {
                        Text(hint)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(CloudotTheme.compactSpacing)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("\(check.level.accessibilityLabel)：\(check.name)。\(check.message)")
        }
    }
}
