import Foundation

extension Check {
    var displayName: String {
        switch name {
        case "config": "Cloudot 配置"
        case "store": "本地存储"
        case "git-identity": "Git 身份"
        case "remote": "远端仓库"
        case "sync-state": "同步状态"
        case let name where name.hasPrefix("link:"): "配置链接"
        case "secrets": "敏感信息"
        case "backups": "备份"
        default: name
        }
    }

    var detailName: String? {
        guard name.hasPrefix("link:") else { return nil }
        return String(name.dropFirst("link:".count))
    }
}
