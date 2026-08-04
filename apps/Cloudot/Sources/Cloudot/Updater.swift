import AppKit
import Foundation

/// 自更新：查版本、下 DMG、替换自己、重启。
///
/// 为什么不用 Sparkle：它要求把 framework 拷进 bundle 并单独签名（Xcode 会代劳，
/// 这个项目是 `make-app.sh` 手工组装的），还要维护 appcast。而更新逻辑本身就这么点，
/// 自己写反而少一层依赖。见 dist/README.md。
///
/// **只有一条路径：下载替换。** 不判断是不是 Homebrew 装的、不在 GUI 里跑 brew ——
/// Homebrew 用户想用 brew 升级就自己在终端跑，那是那条渠道自己的事。
@MainActor
enum Updater {
    /// 更新源。两层覆盖，方便本地验收：
    ///   1. 环境变量 `CLOUDOT_UPDATE_FEED` —— 测试进程和脚本用这个最直接
    ///   2. `defaults write com.tarnish233.cloudot updateFeedURL http://127.0.0.1:8000`
    ///      —— 真 App 用，照 `cloudotBinaryPath` 那个覆盖开关的样子
    /// 都没设就走 GitHub。
    private static var baseURL: String {
        if let env = ProcessInfo.processInfo.environment["CLOUDOT_UPDATE_FEED"], !env.isEmpty {
            return env
        }
        return UserDefaults.standard.string(forKey: "updateFeedURL")
            ?? "https://github.com/tarnish233/cloudot"
    }

    // MARK: - 查

    /// 读最新版本号。
    ///
    /// 走 `releases/latest` 的 **302 重定向**而不是 GitHub API：API 的匿名限额是
    /// 60 次/小时**且按 IP 共享**，NAT 后面的用户可能一次都用不上（实测探测几轮就耗尽了）。
    /// 重定向没有这个限制，`Location` 头里就带着 tag。
    static func check(current: String) async throws -> UpdateCheck {
        guard let currentVersion = SemanticVersion(current) else {
            throw UpdateError.unreadableVersion(current)
        }
        guard let url = URL(string: "\(baseURL)/releases/latest") else {
            throw UpdateError.badFeedURL(baseURL)
        }

        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        request.timeoutInterval = 10

        let session = URLSession(
            configuration: .ephemeral,          // 不留缓存，免得读到过期的重定向
            delegate: RedirectBlocker(),
            delegateQueue: nil
        )
        defer { session.finishTasksAndInvalidate() }

        let response: URLResponse
        do {
            (_, response) = try await session.data(for: request)
        } catch {
            throw UpdateError.network(error)
        }

        guard let http = response as? HTTPURLResponse,
              let location = http.value(forHTTPHeaderField: "Location"),
              let tag = URL(string: location)?.lastPathComponent,
              let latest = SemanticVersion(tag)
        else {
            throw UpdateError.unreadableFeed
        }

        // 资产名跟着 make-app.sh 的命名走：Cloudot-<版本>.dmg
        let versionText = latest.description
        guard let dmg = URL(string: "\(baseURL)/releases/download/v\(versionText)/Cloudot-\(versionText).dmg"),
              let page = URL(string: "\(baseURL)/releases/tag/v\(versionText)")
        else {
            throw UpdateError.badFeedURL(baseURL)
        }

        return UpdateCheck(
            current: currentVersion,
            latest: latest,
            downloadURL: dmg,
            releasePageURL: page
        )
    }

    /// 不跟随重定向，这样才能读到 `Location` 头。
    private final class RedirectBlocker: NSObject, URLSessionTaskDelegate, Sendable {
        func urlSession(
            _ session: URLSession,
            task: URLSessionTask,
            willPerformHTTPRedirection response: HTTPURLResponse,
            newRequest request: URLRequest,
            completionHandler: @escaping (URLRequest?) -> Void
        ) {
            completionHandler(nil)
        }
    }

    // MARK: - 下

    /// 下载 DMG 并校验 sha256，返回落地路径。
    ///
    /// 用 URLSession 而不是 spawn curl：curl 把进度写 stderr 且量大，正好是能触发
    /// `CloudotCLI.execute` 那个管道死锁的形状（那里先把 stdout 读到 EOF 才读 stderr）。
    /// URLSession 没有管道，还自带超时和取消。
    static func download(_ check: UpdateCheck) async throws -> URL {
        let session = URLSession(configuration: .ephemeral)
        defer { session.finishTasksAndInvalidate() }

        let dmg = try await fetch(check.downloadURL, session: session,
                                  missing: .assetMissing(version: check.latest.description))
        // 校验和文件是 make-app.sh 一起产出的，格式是 `<64位hex>  <文件名>`（两个空格）
        let sums = try await fetch(check.downloadURL.appendingPathExtension("sha256"),
                                   session: session,
                                   missing: .assetMissing(version: check.latest.description))

        let directory = FileManager.default.temporaryDirectory
            .appending(path: "cloudot-update-\(check.latest.description)")
        try? FileManager.default.removeItem(at: directory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let dmgPath = directory.appending(path: "Cloudot-\(check.latest.description).dmg")
        try dmg.write(to: dmgPath)

        let expected = String(decoding: sums, as: UTF8.self)
            .split(separator: " ").first.map(String.init) ?? ""
        let actual = try run("/usr/bin/shasum", ["-a", "256", dmgPath.path], timeout: 60)
            .split(separator: " ").first.map(String.init) ?? ""
        guard !expected.isEmpty, expected.lowercased() == actual.lowercased() else {
            try? FileManager.default.removeItem(at: directory)
            throw UpdateError.checksumMismatch
        }
        return dmgPath
    }

    private static func fetch(
        _ url: URL, session: URLSession, missing: UpdateError
    ) async throws -> Data {
        var request = URLRequest(url: url)
        request.timeoutInterval = 120
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw UpdateError.network(error)
        }
        if let http = response as? HTTPURLResponse, http.statusCode == 404 {
            // 发布是手工传的，忘传 DMG 完全可能发生 —— 这种要单独报，
            // 不能混进「网络错误」让用户去查网。
            throw missing
        }
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw UpdateError.unreadableFeed
        }
        return data
    }

    // MARK: - 装

    /// 用 DMG 里的 .app 替换当前 bundle。
    ///
    /// 顺序是刻意的，**任何一步失败旧版都还在**：新版先落到临时位置并校验，确认没问题
    /// 才把旧版挪开。反过来做（先删旧的再拷新的）中途失败就没 app 了。
    ///
    /// 替换正在运行的 bundle 是安全的 —— 实测 `mv` 走整个 .app 之后进程照样跑
    /// （inode 已经打开了），只是要重启才能用上新版。
    ///
    /// `replacing` 默认是正在跑的这个 .app；测试和本地 e2e 会传一个副本路径 ——
    /// `Bundle.main.bundleURL` 在 `swift test` 里指向 toolchain，不能拿来真换。
    ///
    /// **`nonisolated`，必须在主线程之外跑**（调用点用 `Task.detached`）。挂载、
    /// 复制、校验全是同步的子进程等待：`hdiutil` 超时上限 60 秒、`ditto` 120 秒，
    /// 留在主 actor 上就是整个 App 卡住不响应 —— 不只是那个窗口，菜单栏也点不动。
    /// 里面没有任何主 actor 状态（只读 `Bundle.main` 和文件系统），所以搬得动。
    nonisolated static func install(
        dmg: URL,
        expecting: SemanticVersion,
        replacing target: URL = Bundle.main.bundleURL
    ) throws {
        let current = target
        let staging = dmg.deletingLastPathComponent()
        let candidate = staging.appending(path: ".Cloudot.app.new")
        let mount = staging.appending(path: "mnt")

        try? FileManager.default.removeItem(at: candidate)
        try FileManager.default.createDirectory(at: mount, withIntermediateDirectories: true)

        // -readonly 更快也更安全；-nobrowse 不在 Finder 里冒出来打扰人
        _ = try run("/usr/bin/hdiutil",
                    ["attach", dmg.path, "-mountpoint", mount.path,
                     "-nobrowse", "-readonly", "-quiet"],
                    timeout: 60)
        // 挂上了就一定要卸掉，否则会一直占着一个虚拟磁盘
        defer { _ = try? run("/usr/bin/hdiutil", ["detach", mount.path, "-quiet"], timeout: 30) }

        // ditto 而不是 cp -R：它保留扩展属性和签名（实测 adhoc 签名能完整带过来）
        _ = try run("/usr/bin/ditto",
                    [mount.appending(path: "Cloudot.app").path, candidate.path],
                    timeout: 120)

        try verify(candidate, expecting: expecting)

        let backup = current.deletingLastPathComponent().appending(path: ".Cloudot.app.old")
        try? FileManager.default.removeItem(at: backup)
        do {
            try FileManager.default.moveItem(at: current, to: backup)
        } catch {
            throw UpdateError.installFailed("挪开旧版本失败：\(error.localizedDescription)")
        }
        do {
            try FileManager.default.moveItem(at: candidate, to: current)
        } catch {
            // 新版没放进去，把旧版挪回来 —— 否则用户的 app 就凭空消失了
            try? FileManager.default.moveItem(at: backup, to: current)
            throw UpdateError.installFailed("放入新版本失败：\(error.localizedDescription)")
        }
        try? FileManager.default.removeItem(at: backup)
    }

    /// 校验拷出来的 bundle 真的能用、且版本号是期望的那个。
    /// 不做这一步的话，一个损坏或者版本不对的 DMG 会让用户换到一个打不开的 app。
    nonisolated private static func verify(_ bundle: URL, expecting: SemanticVersion) throws {
        let binary = bundle.appending(path: "Contents/MacOS/Cloudot")
        guard FileManager.default.isExecutableFile(atPath: binary.path) else {
            throw UpdateError.installFailed("新版本里没有可执行文件")
        }
        let plist = bundle.appending(path: "Contents/Info.plist")
        guard let data = try? Data(contentsOf: plist),
              let info = try? PropertyListSerialization
                  .propertyList(from: data, format: nil) as? [String: Any],
              let text = info["CFBundleShortVersionString"] as? String,
              let found = SemanticVersion(text)
        else {
            throw UpdateError.installFailed("读不出新版本的版本号")
        }
        guard found == expecting else {
            throw UpdateError.versionMismatch(expected: expecting.description,
                                              got: found.description)
        }
    }

    // MARK: - 重启

    /// 退出自己，让一个脱离出去的子进程把新版拉起来。
    ///
    /// 子进程必须 detached：父进程马上就要退出了，`sleep` 得由别人来等。
    /// 实测父进程退出后子进程照样执行完。
    static func relaunch() {
        let path = Bundle.main.bundleURL.path
        let process = Process()
        process.executableURL = URL(filePath: "/bin/sh")
        process.arguments = ["-c", "sleep 1; open '\(path)'"]
        try? process.run()
        NSApp.terminate(nil)
    }

    // MARK: - 子进程

    /// 起一个外部命令，带超时。
    ///
    /// 不复用 `CloudotCLI.execute`：那个是 private，而且 `CloudotError` 三个 case 全是
    /// cloudot 的 JSON 信封形状，hdiutil 的失败不该从那里解读。
    ///
    /// 这里两个管道**分开线程排空**。`hdiutil` / `ditto` 的输出量实测都在 KB 级
    /// （attach 1.4KB、ditto 0B），本来不至于填满管道；但顺序读的写法一旦被拿去跑
    /// 输出量大的命令就会死锁（实测往 stderr 灌 310KB 就卡住不返回），所以从一开始
    /// 就按安全的写法来。超时也是必须的 —— 损坏的 DMG 能让 `hdiutil attach` 挂住。
    @discardableResult
    nonisolated private static func run(
        _ tool: String, _ arguments: [String], timeout: TimeInterval
    ) throws -> String {
        guard FileManager.default.isExecutableFile(atPath: tool) else {
            throw UpdateError.installFailed("系统里找不到 \(tool)")
        }
        let process = Process()
        process.executableURL = URL(filePath: tool)
        process.arguments = arguments
        let out = Pipe(), err = Pipe()
        process.standardOutput = out
        process.standardError = err

        let collected = Collected()
        let draining = DispatchGroup()
        for (pipe, isStdout) in [(out, true), (err, false)] {
            draining.enter()
            DispatchQueue.global().async {
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                collected.put(data, stdout: isStdout)
                draining.leave()
            }
        }

        do {
            try process.run()
        } catch {
            throw UpdateError.installFailed("\(tool) 起不来：\(error.localizedDescription)")
        }

        let exited = DispatchGroup()
        exited.enter()
        DispatchQueue.global().async {
            process.waitUntilExit()
            exited.leave()
        }
        if exited.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            if exited.wait(timeout: .now() + 2) == .timedOut {
                kill(process.processIdentifier, SIGKILL)
            }
            _ = draining.wait(timeout: .now() + 2)
            throw UpdateError.installFailed(
                "\(URL(filePath: tool).lastPathComponent) 超过 \(Int(timeout)) 秒没有结束")
        }
        _ = draining.wait(timeout: .now() + 5)

        guard process.terminationStatus == 0 else {
            let detail = collected.stderrText.trimmingCharacters(in: .whitespacesAndNewlines)
            throw UpdateError.installFailed(
                "\(URL(filePath: tool).lastPathComponent) 退出码 \(process.terminationStatus)"
                    + (detail.isEmpty ? "" : "：\(detail)"))
        }
        return collected.stdoutText
    }

    /// 两个线程写、主线程读，所以要加锁。
    private final class Collected: @unchecked Sendable {
        private let lock = NSLock()
        private var stdout = Data()
        private var stderr = Data()

        func put(_ data: Data, stdout isStdout: Bool) {
            lock.lock(); defer { lock.unlock() }
            if isStdout { stdout = data } else { stderr = data }
        }

        var stdoutText: String {
            lock.lock(); defer { lock.unlock() }
            return String(decoding: stdout, as: UTF8.self)
        }

        var stderrText: String {
            lock.lock(); defer { lock.unlock() }
            return String(decoding: stderr, as: UTF8.self)
        }
    }
}

/// 更新失败的分类。分开是因为**处置方式不同** —— 网络问题让用户等等再试，
/// 资产缺失得让用户去看发布页，校验和不符则要明确说「不装了」。
enum UpdateError: LocalizedError {
    case network(Error)
    /// 有新版本，但那个版本没传 DMG。发布是手工的，这事完全可能发生。
    case assetMissing(version: String)
    case checksumMismatch
    case versionMismatch(expected: String, got: String)
    case installFailed(String)
    case unreadableFeed
    case unreadableVersion(String)
    case badFeedURL(String)

    var errorDescription: String? {
        switch self {
        case .network(let error):
            "连不上更新服务器：\(error.localizedDescription)"
        case .assetMissing(let version):
            "\(version) 的安装包还没上传，稍后再试或者去发布页看看。"
        case .checksumMismatch:
            "下载的文件校验不通过，可能损坏或被篡改，已经放弃安装。"
        case .versionMismatch(let expected, let got):
            "安装包里是 \(got)，但预期是 \(expected)，已经放弃安装。"
        case .installFailed(let detail):
            "安装失败：\(detail)"
        case .unreadableFeed:
            "读不出最新版本号，更新服务器的响应不是预期的格式。"
        case .unreadableVersion(let text):
            "读不出当前版本号（\(text)）。"
        case .badFeedURL(let text):
            "更新地址不合法：\(text)"
        }
    }

    /// 界面标题用的短摘要，和 `CloudotError.summary` 一个用途。
    var summary: String {
        switch self {
        case .network: "连不上更新服务器"
        case .assetMissing: "安装包还没上传"
        case .checksumMismatch: "安装包校验失败"
        case .versionMismatch: "安装包版本不符"
        case .installFailed: "更新失败"
        case .unreadableFeed, .unreadableVersion, .badFeedURL: "更新检查失败"
        }
    }
}
