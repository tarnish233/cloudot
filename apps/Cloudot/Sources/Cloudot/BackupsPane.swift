import SwiftUI

struct BackupsPane: View {
    let model: AppModel
    private static let keep = 20

    var body: some View {
        Group {
            if let set = model.backups, !set.entries.isEmpty {
                VStack(alignment: .leading, spacing: CloudotTheme.sectionSpacing) {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .center, spacing: CloudotTheme.sectionSpacing) {
                            summary(set)
                            Spacer()
                            pruneButton(set)
                        }

                        VStack(alignment: .leading, spacing: CloudotTheme.compactSpacing) {
                            summary(set)
                            pruneButton(set)
                        }
                    }

                    Label(
                        "备份是配置自愈时的兜底数据源，因此不会自动清理。",
                        systemImage: "info.circle"
                    )
                    .font(.callout)
                    .foregroundStyle(.secondary)

                    Table(set.entries) {
                        TableColumn("时间") { entry in
                            Text(Format.stamp(entry.stamp))
                        }
                        TableColumn("文件数") { entry in
                            Text(entry.files, format: .number)
                        }
                        TableColumn("大小") { entry in
                            Text(Format.bytes(entry.bytes))
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .padding(CloudotTheme.pagePadding)
            } else {
                ContentUnavailableView(
                    "还没有备份",
                    systemImage: "clock.arrow.circlepath",
                    description: Text("同步或覆盖配置时会自动生成备份。")
                )
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func summary(_ set: BackupSet) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("\(set.entries.count) 份备份")
                .font(.headline)
            Text("\(set.totalFiles) 个文件 · \(Format.bytes(set.totalBytes))")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func pruneButton(_ set: BackupSet) -> some View {
        if set.entries.count > Self.keep {
            Button("清理旧备份", role: .destructive) {
                model.pending = .pruneBackups(
                    keep: Self.keep,
                    willRemove: set.entries.count - Self.keep
                )
            }
            .disabled(model.isBusy)
        }
    }
}
