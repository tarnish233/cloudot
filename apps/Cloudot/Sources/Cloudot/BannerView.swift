import SwiftUI

/// 操作反馈条。
struct BannerView: View {
    let banner: Banner
    var compact = false
    let dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: CloudotTheme.compactSpacing) {
            Image(systemName: symbol)
                .foregroundStyle(tint)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 4) {
                Text(banner.title)
                    .font(compact ? .callout : .headline)
                    .bold()

                if let detail = banner.detail, !detail.isEmpty {
                    Text(detail)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(compact ? 4 : nil)
                        .textSelection(.enabled)
                }

                if let hint = banner.terminalHint {
                    Text(hint)
                        .font(.system(.callout, design: .monospaced))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 4)
                        .background(.quaternary, in: .rect(cornerRadius: 5))
                        .textSelection(.enabled)
                }
            }

            Spacer(minLength: 0)

            Button("关闭提示", systemImage: "xmark", action: dismiss)
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .help("关闭提示")
        }
        .padding(compact ? 0 : CloudotTheme.cardPadding)
        .background(
            compact ? AnyShapeStyle(.clear) : AnyShapeStyle(tint.opacity(0.09)),
            in: .rect(cornerRadius: 8)
        )
        .accessibilityElement(children: .contain)
    }

    private var symbol: String {
        switch banner.tone {
        case .success: "checkmark.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .failure: "xmark.octagon.fill"
        }
    }

    private var tint: Color {
        switch banner.tone {
        case .success: SystemTheme.accentColor
        case .warning: .orange
        case .failure: .red
        }
    }
}
