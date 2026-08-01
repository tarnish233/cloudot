import Foundation
import Observation

/// 需要用户点头才执行的破坏性操作。
///
/// `--force` / `--allow-secrets` 这类真能丢数据或泄凭据的开关刻意不进 GUI，
/// 所以这里只有三种：纳管、退出纳管、清理备份。
enum PendingAction: Identifiable {
    case adopt(id: String, name: String)
    case unadopt(id: String, name: String)
    case pruneBackups(keep: Int, willRemove: Int)

    var id: String {
        switch self {
        case .adopt(let id, _): "adopt-\(id)"
        case .unadopt(let id, _): "unadopt-\(id)"
        case .pruneBackups(let keep, _): "prune-\(keep)"
        }
    }

    var title: String {
        switch self {
        case .adopt(_, let name): "同步 \(name)？"
        case .unadopt(_, let name): "停止同步 \(name)？"
        case .pruneBackups: "清理旧备份？"
        }
    }

    var explanation: String {
        switch self {
        case .adopt(_, let name):
            """
            会把 \(name) 的配置文件移进 ~/.cloudot/store，原地留一个软链指过去。
            动手之前会先备份到 ~/.cloudot/backups，之后可以停止同步并还原本地文件。
            """
        case .unadopt(_, let name):
            """
            会把 \(name) 的软链换回实体文件，并从 store 和清单里移除。
            本地配置内容不会丢，但之后不再跨机器同步。
            """
        case .pruneBackups(let keep, let willRemove):
            """
            会删掉 \(willRemove) 份旧备份，保留最近 \(keep) 份。
            备份是配置自愈时的兜底数据源，删掉就不可恢复了。
            """
        }
    }

    var confirmLabel: String {
        switch self {
        case .adopt: "同步"
        case .unadopt: "停止同步"
        case .pruneBackups: "删除"
        }
    }

    var isDestructive: Bool {
        switch self {
        case .adopt: false
        case .unadopt, .pruneBackups: true
        }
    }
}

/// 一次操作之后要给用户看的反馈。
struct Banner: Identifiable {
    enum Tone { case success, warning, failure }

    let id = UUID()
    let tone: Tone
    let title: String
    let detail: String?
    /// 错误分类，界面用它决定要不要附上「去终端做」的引导
    let kind: ErrorKind?
    /// 出错的对象（应用 id）。有它才能把引导命令拼成可直接复制的形式，
    /// 而不是让用户自己把 `<应用>` 换掉。
    let subject: String?

    static func ok(_ title: String, _ detail: String? = nil) -> Banner {
        Banner(tone: .success, title: title, detail: detail, kind: nil, subject: nil)
    }

    static func warn(_ title: String, _ detail: String? = nil) -> Banner {
        Banner(tone: .warning, title: title, detail: detail, kind: nil, subject: nil)
    }

    static func from(_ error: Error, subject: String? = nil) -> Banner {
        if let e = error as? CloudotError {
            return Banner(
                tone: .failure,
                title: e.summary,
                detail: e.errorDescription,
                kind: e.kind,
                subject: subject
            )
        }
        return Banner(
            tone: .failure,
            title: "操作失败",
            detail: error.localizedDescription,
            kind: nil,
            subject: subject
        )
    }

    /// 有些错误只能在终端里收尾（涉及 --force 之类的开关），给出具体命令。
    var terminalHint: String? {
        switch kind {
        case .needsForce:
            "cloudot apply --force    # 会先备份本地那份再用 store 版本覆盖"
        case .secretsDetected:
            "cloudot add \(subject ?? "<应用>") --allow-secrets    # 确认内容无害后再用"
        case .pullConflict:
            "cloudot status && git -C ~/.cloudot/store log -p -1"
        case .notInitialized:
            "cloudot init --remote git@github.com:<你>/dotfiles.git"
        default:
            nil
        }
    }
}

@MainActor
@Observable
final class AppModel {
    private var cli: CloudotCLI?
    private(set) var locateError: CloudotError?

    /// 找到了 cloudot、可以执行命令。
    var isReady: Bool { cli != nil }

    private(set) var status: Status?
    private(set) var doctor: DoctorReport?
    private(set) var appList: [AppListing] = []
    private(set) var backups: BackupSet?

    private(set) var isRefreshing = false
    /// 这次刷新是用户主动点的吗。
    ///
    /// 只有主动刷新才让菜单栏机器人扫眼：后台的只读检查不该在视觉上打扰人，
    /// 而且原来那个动画连一轮都播不完（12 帧 ×80ms 要 0.96s，刷新只要 0.17s），
    /// 看起来就是每 20 秒眼睛歪一下又弹回去，像抽动。
    private var isUserInitiatedRefresh = false
    private(set) var isWorking = false
    private(set) var lastRefresh: Date?

    var isBusy: Bool { isRefreshing || isWorking }

    var banner: Banner?
    var pending: PendingAction?

    var isPresentingPendingAction: Bool {
        get { pending != nil }
        set {
            if !newValue {
                pending = nil
            }
        }
    }

    /// 兜底轮询的间隔。
    ///
    /// 主要的刷新触发是 FSEvents（见 `watcher`）—— 配置文件真的变了才刷。这个轮询
    /// 只是防线：FSEvents 有极少数情况会漏（比如机器休眠期间的变化、监视目录被整个
    /// 移走），一天一次足够把状态兜回来。
    ///
    /// 不能靠轮询发现远端改动：`status` 读的是本地缓存的 `@{upstream}` ref，**不 fetch**。
    static let refreshInterval: Duration = .seconds(24 * 60 * 60)

    private var refreshLoop: Task<Void, Never>?

    /// 配置文件变化的监视器。有变化才刷新，没变化零开销。
    private var watcher: ConfigWatcher?
    /// 上次实际交给 watcher 的目录，用来避免重复重建 stream。
    private var watchedPaths: Set<String> = []
    /// 自己动过文件的时刻。
    ///
    /// `sync` / `apply` / `add` 会真的写 store，FSEvents 因此会报事件 —— 但那些操作
    /// 结束时本来就会刷新一次，watcher 再触发就是白跑一轮。这里记下时间，
    /// 短时间内的事件直接忽略。（`status` 是纯只读，实测不会产生事件。）
    private var lastSelfWrite: Date?
    /// 自己写完之后多久内的文件事件算「自己造成的」。
    private static let selfWriteGrace: TimeInterval = 3

    init() {
        switch CloudotCLI.locate() {
        case .success(let cli): self.cli = cli
        case .failure(let error): self.locateError = error
        }
    }

    /// 启动文件监视 + 兜底轮询。重复调用是安全的。
    func startAutoRefresh() {
        startWatching()
        guard refreshLoop == nil else { return }
        refreshLoop = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: Self.refreshInterval)
            }
        }
    }

    /// 装上 FSEvents 监视。
    ///
    /// 监视的是**目录**而不是具体文件：FSEvents 按目录订阅，而且纳管的文件会被
    /// rename（软链被替换写入顶掉时就是），盯文件本身会丢事件。
    private func startWatching() {
        guard watcher == nil else { return }
        watcher = ConfigWatcher { [weak self] in
            // 回调在后台队列，跳回主 actor 再动 model
            Task { @MainActor [weak self] in
                guard let self, !self.isEchoOfOwnWrite else { return }
                await self.refresh()
            }
        }
        updateWatchedDirectories()
    }

    /// 这次文件事件是不是我们自己刚写出来的回声。
    private var isEchoOfOwnWrite: Bool {
        guard let lastSelfWrite else { return false }
        return Date.now.timeIntervalSince(lastSelfWrite) < Self.selfWriteGrace
    }

    /// 按当前纳管情况更新监视范围。每次 `refresh` 之后调用。
    ///
    /// 纳管新应用之后监视范围要跟着变，否则新纳管的配置改了不会触发刷新。
    private func updateWatchedDirectories() {
        guard let watcher, let status else { return }

        var dirs: Set<String> = []
        // store 那棵树：常规编辑（通过软链写入）和 sync 拉下新内容都只报在这里
        dirs.insert(status.root + "/store/files")
        // 每个纳管文件所在的目录：软链被替换写入顶掉时只报在这里
        for app in status.apps {
            for file in app.files {
                let expanded = NSString(string: file.target).expandingTildeInPath
                dirs.insert((expanded as NSString).deletingLastPathComponent)
            }
        }

        // 目录集合没变就不重建 stream —— 重建会丢掉 sinceNow 之前的事件
        guard dirs != watchedPaths else { return }
        watchedPaths = dirs
        watcher.start(watching: dirs.map { URL(filePath: $0) })
    }

    /// 只刷新 `status`。打开菜单栏面板时调用。
    ///
    /// 为什么打开时要刷：面板上就有「立即同步」按钮，如果显示的是上次轮询时的旧状态，
    /// 就可能一边写着「已同步」一边让你点同步 —— 自相矛盾。`status` 是现读 git 的，
    /// 没有缓存，所以刷一次就是当下的真实状态。
    ///
    /// 只拉 status 而不是整套：面板只用到 `model.status`，而 `doctor` 单独就要 87ms，
    /// 占了整轮刷新的一半 —— 拉了也没人看。
    func refreshStatusOnly() async {
        guard let cli, !isBusy else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        do {
            status = try await cli.status()
            lastRefresh = .now
        } catch {
            banner = .from(error)
        }
    }

    // MARK: - 派生出来给界面用的东西

    /// 菜单栏图标状态。
    ///
    /// 与 `overallLevel` 分开：菜单栏是**单色 template 渲染**，颜色不生效，
    /// 状态只能靠形状传达；而 App 内部可以用颜色，两者的取舍不一样。
    enum IconState {
        /// 一切正常
        case healthy
        /// 有待同步的改动
        case pending
        /// 正在刷新只读状态
        case refreshing
        /// 正在执行操作
        case syncing
        /// 有配置读不到了
        case broken
        /// 找不到 cloudot，读不出状态
        case unavailable
    }

    /// 一次性动效事件（同步成功/失败）。
    ///
    /// 带自增 id 而不是「消费后清空」：菜单栏只需要记住上次播过的 id，
    /// 不用回头改 model 的状态，读写方向保持单一。
    struct IconPulse: Equatable {
        enum Kind { case success, failure }
        let id: Int
        let kind: Kind
    }

    private(set) var pulse: IconPulse?
    private var pulseSeq = 0

    private func emit(_ kind: IconPulse.Kind) {
        pulseSeq += 1
        pulse = IconPulse(id: pulseSeq, kind: kind)
    }

    var iconState: IconState {
        if locateError != nil { return .unavailable }
        if isWorking { return .syncing }
        if isRefreshing && isUserInitiatedRefresh { return .refreshing }
        guard let status else { return .unavailable }
        if !status.healthy { return .broken }
        return status.git.hasPendingWork ? .pending : .healthy
    }

    var overallLevel: Level {
        if locateError != nil { return .error }
        guard let status else { return .warn }
        if !status.healthy { return .error }
        if status.git.hasPendingWork { return .warn }
        return .ok
    }

    var headline: String {
        if locateError != nil { return "找不到 cloudot" }
        guard let status else { return "读取中…" }
        if !status.orphans.isEmpty { return "有配置读不到了" }
        if !status.healthy { return "需要处理" }
        if let behind = status.git.behind, behind > 0 { return "远端有 \(behind) 个新提交" }
        if let ahead = status.git.ahead, ahead > 0 { return "有 \(ahead) 个提交待推送" }
        if !status.git.dirty.isEmpty { return "有未提交的改动" }
        return "已同步"
    }

    /// 是否有 apply（不带 force）能修好的问题。
    var hasApplyableWork: Bool {
        guard let status else { return false }
        if !status.orphans.isEmpty { return true }
        return status.apps.contains { $0.files.contains { $0.state.fixableByApply } }
    }

    var detectedCandidates: [AppListing] {
        appList.filter { !$0.managed && $0.detected }
    }

    var unavailableApps: [AppListing] {
        appList.filter { !$0.managed && !$0.detected }
    }

    // MARK: - 读

    /// - Parameter userInitiated: 用户点了「刷新」按钮时传 true，菜单栏才播动画。
    func refresh(userInitiated: Bool = false) async {
        guard let cli, !isBusy else { return }
        isRefreshing = true
        isUserInitiatedRefresh = userInitiated
        defer {
            isRefreshing = false
            isUserInitiatedRefresh = false
        }

        // 状态是主角，读不到就没必要继续；其余各自失败不影响整体。
        do {
            status = try await cli.status()
        } catch {
            banner = .from(error)
            return
        }
        doctor = try? await cli.doctor(net: false)
        appList = (try? await cli.apps()) ?? []
        backups = try? await cli.backups()
        lastRefresh = .now
        // 纳管范围可能变了（刚 add 完），监视范围要跟上
        updateWatchedDirectories()
    }

    // MARK: - 写

    func sync() async {
        await perform { cli in
            let result = try await cli.sync()
            var parts: [String] = []
            if let commit = result.commit { parts.append("已提交 \(commit)") }
            parts.append(result.pull.label)
            if result.pushed { parts.append("已推送") }
            for healed in result.applied.healed {
                parts.append("\(healed.target)：\(healed.source.label)")
            }
            for item in result.applied.changedItems {
                parts.append("\(item.target)：\(item.action.label)")
            }
            return .ok("同步完成", parts.joined(separator: "\n"))
        }
    }

    func apply() async {
        await perform { cli in
            let result = try await cli.apply()
            let changed = result.changedItems
            if changed.isEmpty && result.healed.isEmpty {
                return .ok("没有需要落地的改动")
            }
            let lines = result.healed.map { "\($0.target)：\($0.source.label)" }
                + changed.map { "\($0.target)：\($0.action.label)" }
            let skipped = changed.filter { $0.action == .skipped }
            return skipped.isEmpty
                ? .ok("落地完成", lines.joined(separator: "\n"))
                : .warn("部分条目跳过了", lines.joined(separator: "\n"))
        }
    }

    func confirm(_ action: PendingAction) async {
        pending = nil
        switch action {
        case .adopt(let id, let name):
            await perform(subject: id) { cli in
                let results = try await cli.add(id)
                let files = results.flatMap(\.files).map(\.target)
                return .ok("\(name) 已加入同步", files.joined(separator: "\n"))
            }
        case .unadopt(let id, let name):
            await perform(subject: id) { cli in
                let result = try await cli.unadopt(id)
                return .ok(
                    "\(name) 已停止同步",
                    result.restored.map { "\($0) 已还原成实体文件" }.joined(separator: "\n")
                )
            }
        case .pruneBackups(let keep, _):
            await perform { cli in
                let result = try await cli.pruneBackups(keep: keep)
                return .ok(
                    "已清理 \(result.removed.count) 份备份",
                    "释放 \(Format.bytes(result.freedBytes)) · 保留 \(result.kept) 份"
                )
            }
        }
    }

    /// 跑一个改动操作，然后一定重新拉状态 —— 界面绝不自己推测新状态。
    ///
    /// `subject` 是这次操作的对象（应用 id）。失败时带进 Banner，引导命令才能拼成
    /// 可直接复制的形式 —— GUI 刻意不提供 `--allow-secrets` / `--force`，
    /// 所以「去终端怎么敲」是这条路唯一的出口，不能让用户自己猜占位符。
    private func perform(
        subject: String? = nil,
        _ body: (CloudotCLI) async throws -> Banner
    ) async {
        guard let cli, !isBusy else { return }
        isWorking = true
        // 改动类操作会写 store，先标记，免得 FSEvents 的回声又触发一轮刷新
        lastSelfWrite = .now
        do {
            let result = try await body(cli)
            banner = result
            switch result.tone {
            case .success: emit(.success)
            case .failure: emit(.failure)
            case .warning: break   // 既不是成功，也不值得演一遍死掉
            }
        } catch {
            banner = .from(error, subject: subject)
            emit(.failure)
        }
        // 操作真正结束的时刻才是抑制窗口的起点
        lastSelfWrite = .now
        isWorking = false
        await refresh()
    }
}

extension Duration {
    /// `Duration` 没有直接读秒数的接口，`components` 拆出来自己算。
    var timeInterval: TimeInterval {
        let (seconds, attoseconds) = components
        return TimeInterval(seconds) + TimeInterval(attoseconds) / 1e18
    }
}

enum Format {
    static func bytes(_ count: Int64) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter.string(fromByteCount: count)
    }

    /// `20260801-120750` → `2026-08-01 12:07`
    static func stamp(_ raw: String) -> String {
        let parser = DateFormatter()
        parser.dateFormat = "yyyyMMdd-HHmmss"
        guard let date = parser.date(from: raw) else { return raw }
        let out = DateFormatter()
        out.dateFormat = "yyyy-MM-dd HH:mm"
        return out.string(from: date)
    }

    static func relative(_ date: Date) -> String {
        if abs(date.timeIntervalSinceNow) < 5 {
            return "刚刚"
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: .now)
    }
}
