import SwiftUI

struct MenuBarMissingBinaryView: View {
    let error: CloudotError

    var body: some View {
        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
            Label(error.summary, systemImage: "xmark.octagon.fill")
                .font(.headline)
                .foregroundStyle(.red)
            Text("把 cloudot 装到 PATH，或放进 App 包的 Resources。")
                .font(.callout)
                .foregroundStyle(.secondary)
            Text("cargo install --path crates/cloudot-cli")
                .font(.system(.callout, design: .monospaced))
                .textSelection(.enabled)
        }
        .padding(CloudotTheme.cardPadding)
        .background(.quaternary, in: .rect(cornerRadius: 12))
    }
}
