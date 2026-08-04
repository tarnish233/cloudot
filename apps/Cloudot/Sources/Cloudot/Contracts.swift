import Foundation

// MARK: - 信封
//
// CLI 的 `--json` 对所有命令（包括出错）都输出同一个信封：
//   { "schema": "cloudot.sync/v1", "ok": true,  "result": { … } }
//   { "schema": "cloudot.error/v1", "ok": false, "result": { kind, summary, message } }
// 所以这里只需要一条解码路径。

struct Envelope<Result: Decodable>: Decodable {
    let schema: String
    let ok: Bool
    let result: Result
}

struct ErrorResult: Decodable {
    let kind: ErrorKind
    let summary: String
    let message: String
    /// 仅 `pull_conflict` 时有：冲突文件列表 + 每文件 diff
    let conflict: ConflictReport?
}

/// 与 Rust 侧 `ErrorKind` 对应。未知取值落到 `other`，这样 CLI 加新分类时旧版界面不会崩。
enum ErrorKind: String, Decodable {
    case notInitialized = "not_initialized"
    case locked
    case secretsDetected = "secrets_detected"
    case needsForce = "needs_force"
    case pullConflict = "pull_conflict"
    case unknownApp = "unknown_app"
    case notDetected = "not_detected"
    case notAdopted = "not_adopted"
    case foreignSymlink = "foreign_symlink"
    case unsupported
    case other

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = ErrorKind(rawValue: raw) ?? .other
    }
}

// MARK: - status

struct Status: Decodable {
    let device: String
    let root: String
    /// 是否已经跑过 `cloudot init`。缺省 true：旧 fixture / 旧 CLI 不带这个字段时按已初始化处理。
    var initialized: Bool = true
    /// store 是 git 仓库、所有纳管文件链接正常、没有孤儿
    let healthy: Bool
    let git: GitInfo
    let apps: [AppStatus]
    let available: [AvailableApp]
    let orphans: [Orphan]

    enum CodingKeys: String, CodingKey {
        case device, root, initialized, healthy, git, apps, available, orphans
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        device = try c.decode(String.self, forKey: .device)
        root = try c.decode(String.self, forKey: .root)
        initialized = try c.decodeIfPresent(Bool.self, forKey: .initialized) ?? true
        healthy = try c.decode(Bool.self, forKey: .healthy)
        git = try c.decode(GitInfo.self, forKey: .git)
        apps = try c.decode([AppStatus].self, forKey: .apps)
        available = try c.decode([AvailableApp].self, forKey: .available)
        orphans = try c.decode([Orphan].self, forKey: .orphans)
    }
}

/// 拉取冲突后的结构化报告（随 error 信封的 `conflict` 字段下发）。
struct ConflictReport: Decodable, Identifiable {
    let branch: String
    let remoteRef: String
    let files: [ConflictFile]

    var id: String { "\(branch)|\(remoteRef)|\(files.count)" }

    enum CodingKeys: String, CodingKey {
        case branch, files
        case remoteRef = "remote_ref"
    }
}

struct ConflictFile: Decodable, Identifiable {
    let path: String
    let diff: String
    let truncated: Bool

    var id: String { path }
}

struct GitInfo: Decodable {
    let repo: Bool
    let branch: String?
    let head: String?
    let remote: String?
    let dirty: [String]
    let ahead: Int?
    let behind: Int?

    var hasPendingWork: Bool {
        !dirty.isEmpty || (ahead ?? 0) > 0 || (behind ?? 0) > 0
    }
}

struct AppStatus: Decodable, Identifiable {
    let id: String
    let name: String
    let adoptedBy: String
    let ok: Bool
    let files: [FileStatus]

    enum CodingKeys: String, CodingKey {
        case id, name, ok, files
        case adoptedBy = "adopted_by"
    }
}

struct FileStatus: Decodable, Identifiable {
    let target: String
    let store: String
    let state: LinkState

    var id: String { target }
}

enum LinkState: String, Decodable {
    case linked
    case storeMissing = "store_missing"
    case missing
    case replacedByFile = "replaced_by_file"
    case foreignSymlink = "foreign_symlink"

    var label: String {
        switch self {
        case .linked: "已链接"
        case .storeMissing: "store 内容缺失"
        case .missing: "本地缺失，待落地"
        case .replacedByFile: "本地是实体文件，未链接"
        case .foreignSymlink: "本地软链指向别处"
        }
    }

    var isOK: Bool { self == .linked }

    /// 这个状态能不能靠不带 --force 的 apply 修好。
    var fixableByApply: Bool { self == .missing }
}

struct AvailableApp: Decodable, Identifiable {
    let id: String
    let name: String
}

struct Orphan: Decodable, Identifiable {
    let app: String
    let target: String
    let store: String
    let kind: OrphanKind

    var id: String { target }
}

enum OrphanKind: String, Decodable {
    case dangling
    case unmanaged

    var label: String {
        switch self {
        case .dangling: "悬空软链：store 里的文件已不存在，配置读不到了"
        case .unmanaged: "已不在清单中，但软链仍指向 store"
        }
    }
}

// MARK: - doctor

struct DoctorReport: Decodable {
    let ok: Bool
    let checks: [Check]
}

struct Check: Decodable, Identifiable {
    let name: String
    let level: Level
    let message: String
    let hint: String?

    var id: String { name }
}

enum Level: String, Decodable, Comparable {
    case ok, warn, error

    private var rank: Int {
        switch self {
        case .ok: 0
        case .warn: 1
        case .error: 2
        }
    }

    static func < (a: Level, b: Level) -> Bool { a.rank < b.rank }
}

// MARK: - apps

struct AppListing: Decodable, Identifiable {
    let id: String
    let name: String
    let detected: Bool
    let managed: Bool
}

// MARK: - show

/// `cloudot show <app>`：一个应用的定义 + 每个路径的当前状态。
///
/// 纳管前用它回答「会动我哪些文件」—— 确认框里那份清单就来自这里。
struct ShowResult: Decodable {
    let id: String
    let name: String
    let detected: Bool
    let managed: Bool
    let adoptedBy: String?
    let detect: [String]
    let paths: [ShowPath]

    enum CodingKeys: String, CodingKey {
        case id, name, detected, managed, detect, paths
        case adoptedBy = "adopted_by"
    }
}

struct ShowPath: Decodable, Identifiable {
    let target: String
    /// store 内相对位置。算不出来（家目录之外等）时为 nil，那种路径纳管不了。
    let store: String?
    let state: LinkState
    let exists: Bool

    var id: String { target }
}

// MARK: - backups

struct BackupSet: Decodable {
    let entries: [BackupEntry]
    let totalFiles: Int
    let totalBytes: Int64

    enum CodingKeys: String, CodingKey {
        case entries
        case totalFiles = "total_files"
        case totalBytes = "total_bytes"
    }
}

struct BackupEntry: Decodable, Identifiable {
    let stamp: String
    let files: Int
    let bytes: Int64

    var id: String { stamp }
}

struct PruneResult: Decodable {
    let removed: [BackupEntry]
    let kept: Int
    let freedBytes: Int64

    enum CodingKeys: String, CodingKey {
        case removed, kept
        case freedBytes = "freed_bytes"
    }
}

// MARK: - 改动类命令的结果

struct SyncResult: Decodable {
    let commit: String?
    let pull: PullOutcome
    let pushed: Bool
    let remote: String?
    let applied: ApplyResult
}

enum PullOutcome: String, Decodable {
    case skipped
    case upToDate = "up_to_date"
    case updated

    var label: String {
        switch self {
        case .skipped: "跳过拉取"
        case .upToDate: "已是最新"
        case .updated: "已拉取远端改动"
        }
    }
}

struct ApplyResult: Decodable {
    let items: [ApplyItem]
    let healed: [HealItem]

    /// 真正发生了改变的条目，用来决定要不要在界面上提一句。
    var changedItems: [ApplyItem] { items.filter { $0.action != .alreadyLinked } }

    /// 有没有需要用户知道的未完成事项。
    ///
    /// 两类都算：`skipped`（本地是实体文件、软链指向别处之类，cloudot 刻意没动）
    /// 和 `HealSource.failed`（悬空软链没修好 —— 那意味着**配置此刻读不到**，
    /// 比 skipped 更严重）。
    ///
    /// 放在契约类型上而不是各调用点自己判断：`sync` 和 `apply` 都要用，
    /// 之前就是因为两处各写一遍，`apply` 查了 skipped 而 `sync` 什么都没查，
    /// 于是同一份结果在两条路径上一个报警告、一个报成功。
    var needsAttention: Bool {
        changedItems.contains { $0.action == .skipped }
            || healed.contains { $0.source == .failed }
    }
}

struct ApplyItem: Decodable, Identifiable {
    let target: String
    let before: LinkState
    let action: ApplyAction
    let backup: String?
    let note: String?

    var id: String { target }
}

enum ApplyAction: String, Decodable {
    case alreadyLinked = "already_linked"
    case linked
    case replaced
    case skipped

    var label: String {
        switch self {
        case .alreadyLinked: "已链接"
        case .linked: "已建链"
        case .replaced: "已用 store 版本覆盖"
        case .skipped: "跳过"
        }
    }
}

struct HealItem: Decodable, Identifiable {
    let app: String
    let target: String
    let kind: OrphanKind
    let source: HealSource
    let note: String?

    var id: String { target }
}

enum HealSource: String, Decodable {
    case gitHistory = "git_history"
    case store
    case backup
    case failed

    var label: String {
        switch self {
        case .gitHistory: "已从 git 历史取回内容并还原"
        case .store: "已还原成实体文件"
        case .backup: "已从备份取回内容并还原"
        case .failed: "修复失败，软链保持原样"
        }
    }
}

struct AddResult: Decodable {
    let id: String
    let name: String
    let files: [AddedFile]
    let commit: String?
}

struct AddedFile: Decodable, Identifiable {
    let target: String
    let store: String
    let backup: String?

    var id: String { target }
}

struct UnadoptResult: Decodable {
    let id: String
    let name: String
    let restored: [String]
    let commit: String?
}

struct InitResult: Decodable {
    let root: String
    let device: String
    let remote: String?
    let cloned: Bool
    let already: Bool
    let appsInStore: Int

    enum CodingKeys: String, CodingKey {
        case root, device, remote, cloned, already
        case appsInStore = "apps_in_store"
    }
}

struct ResolveResult: Decodable {
    let side: ResolveSide
    let target: String
    let applied: ApplyResult?
    let head: String?
}

enum ResolveSide: String, Decodable {
    case theirs
    case ours
}
