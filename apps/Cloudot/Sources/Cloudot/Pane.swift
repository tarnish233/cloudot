import Foundation

enum Pane: String, CaseIterable, Hashable, Identifiable {
    case overview, apps, doctor, backups

    var id: Self { self }

    var title: String {
        switch self {
        case .overview: "概览"
        case .apps: "应用"
        case .doctor: "体检"
        case .backups: "备份"
        }
    }

    var subtitle: String {
        switch self {
        case .overview: "同步状态与本机配置概况"
        case .apps: "查看和管理已发现的应用配置"
        case .doctor: "检查配置、仓库与运行环境"
        case .backups: "查看自愈所依赖的本地备份"
        }
    }

    var accessibilityHint: String {
        "显示\(title)页面"
    }

    var symbol: String {
        switch self {
        case .overview: "square.grid.2x2"
        case .apps: "app.badge"
        case .doctor: "stethoscope"
        case .backups: "clock.arrow.circlepath"
        }
    }
}
