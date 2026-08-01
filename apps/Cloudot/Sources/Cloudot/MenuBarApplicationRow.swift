import SwiftUI

struct MenuBarApplicationRow: View {
    let app: AppStatus

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: app.ok ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
                .foregroundStyle(app.ok ? SystemTheme.accentColor : .orange)
                .font(.body)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 1) {
                Text(app.name)
                    .font(.body)
                    .bold()
                Text("\(app.files.count) 个配置文件")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: CloudotTheme.compactSpacing)

            Text(summary)
                .font(.callout)
                .foregroundStyle(
                    app.ok ? AnyShapeStyle(.secondary) : AnyShapeStyle(.orange)
                )
                .lineLimit(1)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, CloudotTheme.compactSpacing)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(app.name)，\(summary)，\(app.files.count) 个配置文件")
    }

    private var summary: String {
        guard !app.ok else { return "正常" }
        return app.files.first(where: { !$0.state.isOK })?.state.label ?? "需要处理"
    }
}
