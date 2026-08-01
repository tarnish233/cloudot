import SwiftUI

struct OverviewStatusCard: View {
    let headline: String
    let level: Level
    let device: String
    let showsApplyAction: Bool
    let isBusy: Bool
    let onApply: () -> Void

    var body: some View {
        GroupBox {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .center, spacing: CloudotTheme.cardPadding) {
                    identity
                    Spacer(minLength: CloudotTheme.sectionSpacing)
                    applyButton
                }

                VStack(alignment: .leading, spacing: CloudotTheme.cardPadding) {
                    identity
                    applyButton
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(CloudotTheme.compactSpacing)
        }
    }

    private var identity: some View {
        HStack(alignment: .center, spacing: CloudotTheme.cardPadding) {
            Image(systemName: level.symbol)
                .font(.title2)
                .foregroundStyle(level.tint)
                .frame(width: 44, height: 44)
                .background(level.tint.opacity(0.12), in: .circle)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 4) {
                Text(headline)
                    .font(.title2.bold())

                Label(device, systemImage: "desktopcomputer")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(headline)，设备 \(device)")
    }

    @ViewBuilder
    private var applyButton: some View {
        if showsApplyAction {
            Button("落地到本机", systemImage: "arrow.down.circle", action: onApply)
                .buttonStyle(.borderedProminent)
                .disabled(isBusy)
        }
    }
}
