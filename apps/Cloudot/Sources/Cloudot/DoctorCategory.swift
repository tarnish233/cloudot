import Foundation

enum DoctorCategory: Int, CaseIterable, Identifiable {
    case configuration
    case repository
    case synchronization
    case files
    case security
    case backups
    case other

    var id: Self { self }

    var title: String {
        switch self {
        case .configuration: "配置"
        case .repository: "仓库"
        case .synchronization: "同步"
        case .files: "配置文件"
        case .security: "安全"
        case .backups: "备份"
        case .other: "其他"
        }
    }

    var symbol: String {
        switch self {
        case .configuration: "gearshape"
        case .repository: "externaldrive"
        case .synchronization: "arrow.triangle.2.circlepath"
        case .files: "doc.on.doc"
        case .security: "lock.shield"
        case .backups: "clock.arrow.circlepath"
        case .other: "ellipsis.circle"
        }
    }

    func checks(in report: DoctorReport) -> [Check] {
        report.checks
            .filter { Self.category(for: $0) == self }
            .sorted { $0.level > $1.level }
    }

    static func category(for check: Check) -> DoctorCategory {
        switch check.name {
        case "config": .configuration
        case "store", "git-identity", "remote": .repository
        case "sync-state": .synchronization
        case let name where name.hasPrefix("link:"): .files
        case "secrets": .security
        case "backups": .backups
        default: .other
        }
    }
}
