import SwiftUI

struct DoctorCheckRow: View {
    let check: Check

    var body: some View {
        HStack(alignment: .top, spacing: CloudotTheme.cardPadding) {
            Image(systemName: check.level.symbol)
                .foregroundStyle(check.level.tint)
                .font(.title3)
                .frame(width: 22)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 4) {
                Text(check.displayName)
                    .font(.headline)
                if let detail = check.detailName {
                    Text(detail)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
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
        .padding(.horizontal, CloudotTheme.compactSpacing)
        .padding(.vertical, 10)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(check.level.accessibilityLabel)：\(check.displayName)。\(check.message)")
    }
}
