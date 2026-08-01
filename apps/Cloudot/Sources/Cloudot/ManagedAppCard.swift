import SwiftUI

struct ManagedAppCard: View {
    let app: AppStatus
    let model: AppModel

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .center, spacing: CloudotTheme.compactSpacing) {
                        identity
                        Spacer()
                        unadoptButton
                    }

                    VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                        identity
                        unadoptButton
                    }
                }

                Divider()

                ForEach(app.files) { file in
                    HStack(alignment: .firstTextBaseline, spacing: CloudotTheme.compactSpacing) {
                        Image(systemName: file.state.isOK ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
                            .foregroundStyle(file.state.isOK ? SystemTheme.accentColor : .orange)
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(file.target)
                                .font(.system(.callout, design: .monospaced))
                                .textSelection(.enabled)
                            Text(file.state.label)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("\(file.target)，\(file.state.label)")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(CloudotTheme.compactSpacing)
        }
    }

    private var identity: some View {
        HStack(spacing: CloudotTheme.compactSpacing) {
            Image(systemName: app.ok ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
                .foregroundStyle(app.ok ? SystemTheme.accentColor : .orange)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(app.name)
                    .font(.headline)
                Text("由 \(app.adoptedBy) 纳管")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(app.name)，\(app.ok ? "状态正常" : "需要处理")，由 \(app.adoptedBy) 纳管")
    }

    private var unadoptButton: some View {
        Button("退出纳管", role: .destructive) {
            model.pending = .unadopt(id: app.id, name: app.name)
        }
        .disabled(model.isBusy)
    }
}
