import CoreServices
import Foundation

/// 监视被纳管配置的文件变化，有变化才通知刷新。
///
/// 取代原来的定时轮询：轮询读的是本地缓存的 `@{upstream}` ref，发现不了远端改动，
/// 唯一能新发现的就是「本机改了配置」—— 而那件事文件系统会主动告诉我们。
///
/// **必须同时监视两处**（这是实测出来的，不是猜的）：
///
/// | 场景 | 事件报在哪 |
/// |---|---|
/// | 通过软链改配置（常规编辑） | 只报 **store**，`~/.config` 那边一个事件都没有 |
/// | 替换写入顶掉软链（karabiner GUI 保存） | 只报 **`~/.config`**，带 `isSymlink` |
/// | `sync` 拉下远端新内容 | 只报 **store** |
///
/// 所以只看 store 会漏掉「软链被顶掉」这个最需要报警的情况，只看 `~/.config`
/// 则连普通的改配置都发现不了。
final class ConfigWatcher {
    /// 事件合并窗口。
    ///
    /// 一次保存往往产生多个事件（写临时文件 → rename → 改属性），而且编辑器
    /// 常常连续写好几次。1 秒足够把一次「保存动作」收成一个通知，又不会让用户
    /// 觉得界面反应慢。
    private static let latency: CFTimeInterval = 1.0

    private var stream: FSEventStreamRef?
    private let onChange: @Sendable () -> Void

    /// - Parameter onChange: 有变化时调用。可能在任意队列上、可能连续多次。
    init(onChange: @escaping @Sendable () -> Void) {
        self.onChange = onChange
    }

    deinit {
        // 这里不能碰 stream 之外的东西：deinit 不保证在哪个线程跑。
        if let stream {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
        }
    }

    /// 开始监视。重复调用是安全的（先停掉旧的）。
    ///
    /// - Parameter directories: 要监视的目录。传目录而不是具体文件：FSEvents 是
    ///   按目录订阅的，而且纳管的文件可能被删掉再建（rename 就是这样），
    ///   盯着文件本身会丢事件。
    func start(watching directories: [URL]) {
        stop()

        // 解析软链：FSEvents 回报的是解析后的真实路径（`/tmp` 会变成 `/private/tmp`），
        // 传进去的路径也解析一遍，两边才对得上。
        let paths = directories
            .map { $0.resolvingSymlinksInPath().path() }
            .filter { FileManager.default.fileExists(atPath: $0) }
        guard !paths.isEmpty else { return }

        // 回调是 C 函数指针，捕获不了 self —— 用 Unmanaged 把 self 塞进 context。
        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil,
            release: nil,
            copyDescription: nil
        )

        let callback: FSEventStreamCallback = { _, info, _, _, _, _ in
            guard let info else { return }
            let watcher = Unmanaged<ConfigWatcher>.fromOpaque(info).takeUnretainedValue()
            // 不解析具体是哪个文件变了：cloudot 的 `status` 本来就是全量现读 git，
            // 知道「有东西变了」就够了。少一层解析，少一个出错的地方。
            watcher.onChange()
        }

        guard let stream = FSEventStreamCreate(
            nil,
            callback,
            &context,
            paths as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            Self.latency,
            // FileEvents：要文件级事件，否则只报目录，分不清是不是纳管的文件变了。
            // WatchRoot：被监视的目录本身被移走/改名时也要知道 —— 那时候纳管已经废了。
            UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagWatchRoot)
        ) else {
            return
        }

        // 走 dispatch queue 而不是 run loop：不依赖某个特定线程的 run loop 在跑，
        // 而 App 里的 run loop 在菜单/面板 tracking 时行为会变。
        FSEventStreamSetDispatchQueue(stream, DispatchQueue.global(qos: .utility))
        FSEventStreamStart(stream)
        self.stream = stream
    }

    func stop() {
        guard let stream else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
        self.stream = nil
    }

    var isWatching: Bool { stream != nil }
}
