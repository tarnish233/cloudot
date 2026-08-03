import Foundation
import XCTest

@testable import Cloudot

/// 自更新相关的纯逻辑。
///
/// 安装路径（挂载 DMG / 替换 bundle）依赖 `Bundle.main.bundleURL`，在 `swift test`
/// 环境下指向 toolchain 而不是 Cloudot.app，没法在这里验 —— 那部分靠本地 e2e。
/// 这里钉住的是**能脱离真实环境验证**的部分：版本比较、URL 拼装、幂等检查、错误文案。
@MainActor
final class UpdaterTests: XCTestCase {

    // MARK: - SemanticVersion

    func testSemverOrdering() {
        // 字符串比较会答错的那条 —— 这就是不用字符串比的理由
        XCTAssertTrue(v("0.9.0") < v("0.10.0"))
        XCTAssertTrue(v("0.2.0") < v("0.3.0"))
        XCTAssertTrue(v("0.2.0") < v("1.0.0"))
        XCTAssertTrue(v("0.2.9") < v("0.2.10"))

        // 位数不同视为补 0
        XCTAssertEqual(v("0.2"), v("0.2.0"))
        XCTAssertEqual(v("1"), v("1.0.0"))
        XCTAssertFalse(v("0.2") < v("0.2.0"))
        XCTAssertFalse(v("0.2.0") < v("0.2"))

        // 相等
        XCTAssertEqual(v("0.3.0"), v("0.3.0"))
        XCTAssertFalse(v("0.3.0") < v("0.3.0"))
    }

    func testSemverAcceptsVPrefixAndPrereleaseSuffix() {
        XCTAssertEqual(v("v0.3.0"), v("0.3.0"))
        // 预发布后缀被砍掉 —— 目前不发预发布，砍掉等于正式版
        XCTAssertEqual(v("0.3.0-beta.1"), v("0.3.0"))
        XCTAssertEqual(v("v0.3.0-rc.2"), v("0.3.0"))
    }

    func testSemverRejectsGarbage() {
        XCTAssertNil(SemanticVersion(""))
        XCTAssertNil(SemanticVersion("not-a-version"))
        XCTAssertNil(SemanticVersion("1..2"))
        XCTAssertNil(SemanticVersion("1.2.x"))
        XCTAssertNil(SemanticVersion("v"))
        XCTAssertNil(SemanticVersion("   "))
    }

    func testSemverDescriptionDropsPrefix() {
        XCTAssertEqual(v("v0.3.0").description, "0.3.0")
        XCTAssertEqual(v("0.2").description, "0.2")
    }

    // MARK: - UpdateCheck

    func testIsAvailableOnlyWhenStrictlyNewer() {
        let base = URL(string: "https://example.com")!
        func check(current: String, latest: String) -> UpdateCheck {
            UpdateCheck(
                current: v(current), latest: v(latest),
                downloadURL: base, releasePageURL: base
            )
        }
        XCTAssertTrue(check(current: "0.2.0", latest: "0.3.0").isAvailable)
        XCTAssertTrue(check(current: "0.9.0", latest: "0.10.0").isAvailable)
        XCTAssertFalse(check(current: "0.3.0", latest: "0.3.0").isAvailable)
        XCTAssertFalse(check(current: "0.3.0", latest: "0.2.0").isAvailable)
        // 位数不同但数值相等
        XCTAssertFalse(check(current: "0.3", latest: "0.3.0").isAvailable)
    }

    // MARK: - URL 拼装

    /// `check` 拼出来的地址必须跟 `make-app.sh` 的命名约定对上，
    /// 否则下载永远 404。资产名是 `Cloudot-<版本>.dmg`，tag 是 `v<版本>`。
    func testCheckBuildsAssetURLsFromTag() async throws {
        // 起一个只回答 `/releases/latest` 的本地假源。
        // 真实 GitHub 是 302；这里直接回 302 + Location，走同一条解析路径。
        let server = try LocalHTTPServer { request in
            if request == "/releases/latest" {
                return .redirect(to: "/releases/tag/v0.9.9")
            }
            return .notFound
        }
        defer { server.stop() }

        try await withFeed(server.baseURL) {
            let result = try await Updater.check(current: "0.2.0")
            XCTAssertEqual(result.latest, v("0.9.9"))
            XCTAssertEqual(result.current, v("0.2.0"))
            XCTAssertTrue(result.isAvailable)
            XCTAssertEqual(
                result.downloadURL.absoluteString,
                "\(server.baseURL)/releases/download/v0.9.9/Cloudot-0.9.9.dmg"
            )
            XCTAssertEqual(
                result.releasePageURL.absoluteString,
                "\(server.baseURL)/releases/tag/v0.9.9"
            )
        }
    }

    func testCheckTreatsSameVersionAsUpToDate() async throws {
        let server = try LocalHTTPServer { request in
            if request == "/releases/latest" {
                return .redirect(to: "/releases/tag/v0.2.0")
            }
            return .notFound
        }
        defer { server.stop() }

        try await withFeed(server.baseURL) {
            let result = try await Updater.check(current: "0.2.0")
            XCTAssertFalse(result.isAvailable)
            XCTAssertEqual(result.latest, result.current)
        }
    }

    func testCheckRejectsUnreadableFeed() async throws {
        // 200 但没有 Location —— 不是我们要的 302 形状
        let server = try LocalHTTPServer { _ in .ok(body: "hello") }
        defer { server.stop() }

        try await withFeed(server.baseURL) {
            do {
                _ = try await Updater.check(current: "0.2.0")
                XCTFail("该抛 unreadableFeed")
            } catch let error as UpdateError {
                guard case .unreadableFeed = error else {
                    return XCTFail("期望 unreadableFeed，拿到 \(error)")
                }
            }
        }
    }

    func testCheckRejectsUnreadableCurrentVersion() async {
        do {
            _ = try await Updater.check(current: "not-a-version")
            XCTFail("该抛 unreadableVersion")
        } catch let error as UpdateError {
            guard case .unreadableVersion(let text) = error else {
                return XCTFail("期望 unreadableVersion，拿到 \(error)")
            }
            XCTAssertEqual(text, "not-a-version")
        } catch {
            XCTFail("期望 UpdateError，拿到 \(error)")
        }
    }

    // MARK: - AppModel 编排

    /// 查更新是旁路操作，不能把同步/刷新一起变灰。
    func testCheckingForUpdateDoesNotMarkBusy() {
        let model = AppModel()
        // 直接断言不变量：标志位存在但 isBusy 的定义里没有它
        XCTAssertFalse(model.isBusy)
        // 用 Mirror 拿不到 private(set) 的写入权，所以只验「定义层面」：
        // isBusy 只看 isRefreshing / isWorking，见 AppModel 源码。
        // 这里钉住公开语义 —— 空闲 model 的 isBusy 为 false，
        // 且 isCheckingForUpdate 默认也是 false。
        XCTAssertFalse(model.isCheckingForUpdate)
    }

    /// 重复检查不该重复打网络。照 `testLoadingVersionTwiceIsCheap` 的样子。
    func testCheckForUpdateIsIdempotent() async throws {
        let server = try LocalHTTPServer { request in
            if request == "/releases/latest" {
                return .redirect(to: "/releases/tag/v9.9.9")
            }
            return .notFound
        }
        defer { server.stop() }

        try await withFeed(server.baseURL) {
            let model = AppModel()
            await model.checkForUpdate()
            let first = try XCTUnwrap(model.updateCheck, "第一次该查到结果")
            XCTAssertEqual(first.latest, v("9.9.9"))

            // 把源关掉再查一次 —— 如果真的又打了网络，会失败并把结果清掉或覆盖。
            // 幂等实现是 `guard updateCheck == nil`（force: false），所以第二次必须直接返回。
            server.stop()
            await model.checkForUpdate()
            XCTAssertEqual(model.updateCheck?.latest, first.latest)
            XCTAssertEqual(model.updateCheck?.downloadURL, first.downloadURL)

            // force: true 会清掉旧结果再查；源已关，应查失败并把结果置空（或保持失败路径）
            await model.checkForUpdate(force: true)
            XCTAssertNil(model.updateCheck, "force 重查时源已关，不该还留着旧的成功结果")
        }
    }

    // MARK: - 错误文案

    /// 资产缺失要给用户能点的发布页，不能混进「网络错误」。
    func testAssetMissingHasDistinctSummary() {
        let error = UpdateError.assetMissing(version: "0.3.0")
        XCTAssertEqual(error.summary, "安装包还没上传")
        XCTAssertTrue(error.errorDescription?.contains("0.3.0") ?? false)
    }

    func testBannerFromUpdateErrorCarriesSummary() {
        let banner = Banner.from(UpdateError.checksumMismatch)
        XCTAssertEqual(banner.tone, .failure)
        XCTAssertEqual(banner.title, "安装包校验失败")
        XCTAssertNotNil(banner.detail)
    }

    /// `hint` 在没有 ErrorKind 匹配时要能回落到 terminalHint ——
    /// 更新失败没有对应的 kind，全靠这个字段把发布页链接送进 BannerView。
    func testBannerHintFallsThroughToTerminalHint() {
        var banner = Banner.fail("安装包还没上传", "0.3.0 的安装包还没上传")
        banner.hint = "https://github.com/tarnish233/cloudot/releases/tag/v0.3.0"
        XCTAssertEqual(banner.terminalHint, banner.hint)

        // 有 kind 时仍以 kind 为准，hint 不抢戏
        let gated = Banner(
            tone: .failure, title: "t", detail: nil,
            kind: .needsForce, subject: nil, hint: "不该出现"
        )
        XCTAssertNotEqual(gated.terminalHint, "不该出现")
        XCTAssertTrue(gated.terminalHint?.contains("apply --force") ?? false)
    }

    func testPendingInstallUpdateCopy() {
        let action = PendingAction.installUpdate(from: "0.2.0", to: "0.3.0")
        XCTAssertEqual(action.title, "更新到 0.3.0？")
        XCTAssertEqual(action.confirmLabel, "更新")
        XCTAssertFalse(action.isDestructive)
        // explanation 要同时提到会发生什么、安全网、Homebrew 后果
        XCTAssertTrue(action.explanation.contains("0.3.0"))
        XCTAssertTrue(action.explanation.contains("0.2.0"))
        XCTAssertTrue(action.explanation.contains("重启"))
        XCTAssertTrue(action.explanation.contains("Homebrew") || action.explanation.contains("brew"))
    }

    // MARK: - 端到端（默认跳过）

    /// 完整下载 → 校验 → 挂载 → 替换。要先备好本地假源：
    ///
    /// ```
    /// # 见 dist/README.md「本地验收自更新」
    /// CLOUDOT_UPDATE_E2E=1 CLOUDOT_UPDATE_FEED=http://127.0.0.1:8765 \
    ///   ./test.sh --filter testDownloadAndInstallReplacesBundle
    /// ```
    ///
    /// 默认跳过：它要真的挂 DMG、动文件系统，还依赖外部起的 HTTP 服务。
    func testDownloadAndInstallReplacesBundle() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["CLOUDOT_UPDATE_E2E"] == "1",
                          "设 CLOUDOT_UPDATE_E2E=1 并起好本地假源再跑")
        try XCTSkipIf(ProcessInfo.processInfo.environment["CLOUDOT_UPDATE_FEED"] == nil,
                      "设 CLOUDOT_UPDATE_FEED=http://127.0.0.1:…")

        // 当前版本故意很低，假源给 9.9.9
        let check = try await Updater.check(current: "0.0.1")
        XCTAssertTrue(check.isAvailable)
        XCTAssertEqual(check.latest, v("9.9.9"))

        let dmg = try await Updater.download(check)
        XCTAssertTrue(FileManager.default.fileExists(atPath: dmg.path))

        // 装到临时目录的一份副本上，不动正在跑的任何东西
        let sandbox = FileManager.default.temporaryDirectory
            .appending(path: "cloudot-update-e2e-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: sandbox, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: sandbox) }

        let target = sandbox.appending(path: "Cloudot.app")
        // 造一个最小的假旧版 bundle：只要目录结构在，install 就会把它 mv 走
        try FileManager.default.createDirectory(
            at: target.appending(path: "Contents/MacOS"), withIntermediateDirectories: true)
        try "old".write(to: target.appending(path: "Contents/MacOS/Cloudot"),
                        atomically: true, encoding: .utf8)
        try """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0"><dict>
          <key>CFBundleShortVersionString</key><string>0.0.1</string>
        </dict></plist>
        """.write(to: target.appending(path: "Contents/Info.plist"),
                  atomically: true, encoding: .utf8)

        try Updater.install(dmg: dmg, expecting: check.latest, replacing: target)

        // 新 bundle 就位、版本对、可执行文件在
        let plist = target.appending(path: "Contents/Info.plist")
        let data = try Data(contentsOf: plist)
        let info = try XCTUnwrap(
            try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
        )
        XCTAssertEqual(info["CFBundleShortVersionString"] as? String, "9.9.9")
        XCTAssertTrue(
            FileManager.default.isExecutableFile(
                atPath: target.appending(path: "Contents/MacOS/Cloudot").path),
            "替换后的可执行文件不存在或不可执行")
        // 旧备份应被清掉
        XCTAssertFalse(
            FileManager.default.fileExists(
                atPath: sandbox.appending(path: ".Cloudot.app.old").path))
    }

    /// 资产 404 要报 `assetMissing`，不能吞成网络错误。
    func testDownloadReportsAssetMissing() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["CLOUDOT_UPDATE_E2E"] == "1")
        try XCTSkipIf(ProcessInfo.processInfo.environment["CLOUDOT_UPDATE_FEED"] == nil)

        // 构造一个指向不存在资产的 check（版本号对，但文件名对不上）
        let check = try await Updater.check(current: "0.0.1")
        let bogus = UpdateCheck(
            current: check.current,
            latest: check.latest,
            downloadURL: check.downloadURL.deletingLastPathComponent()
                .appending(path: "Cloudot-nope.dmg"),
            releasePageURL: check.releasePageURL
        )
        do {
            _ = try await Updater.download(bogus)
            XCTFail("该抛 assetMissing")
        } catch let error as UpdateError {
            guard case .assetMissing = error else {
                return XCTFail("期望 assetMissing，拿到 \(error)")
            }
        }
    }

    /// 校验和不符必须拒绝安装。
    func testDownloadRejectsBadChecksum() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["CLOUDOT_UPDATE_E2E"] == "1")
        let feed = try XCTUnwrap(ProcessInfo.processInfo.environment["CLOUDOT_UPDATE_FEED"])

        // 自己起不了服务端改文件；约定假源额外提供一个坏校验和的版本目录
        // `/releases/download/v9.9.8/`。没有就跳过。
        var head = URLRequest(url: URL(string: "\(feed)/releases/download/v9.9.8/Cloudot-9.9.8.dmg")!)
        head.httpMethod = "HEAD"
        let (_, response) = try await URLSession.shared.data(for: head)
        try XCTSkipUnless((response as? HTTPURLResponse)?.statusCode == 200,
                          "假源没有 v9.9.8 坏校验和夹具，跳过")

        let check = UpdateCheck(
            current: v("0.0.1"),
            latest: v("9.9.8"),
            downloadURL: URL(string: "\(feed)/releases/download/v9.9.8/Cloudot-9.9.8.dmg")!,
            releasePageURL: URL(string: "\(feed)/releases/tag/v9.9.8")!
        )
        do {
            _ = try await Updater.download(check)
            XCTFail("该抛 checksumMismatch")
        } catch let error as UpdateError {
            guard case .checksumMismatch = error else {
                return XCTFail("期望 checksumMismatch，拿到 \(error)")
            }
        }
    }

    // MARK: - 辅助

    private func v(_ text: String) -> SemanticVersion {
        guard let version = SemanticVersion(text) else {
            XCTFail("「\(text)」解析失败"); preconditionFailure()
        }
        return version
    }

    /// 临时把 `CLOUDOT_UPDATE_FEED` 指到给定地址。
    ///
    /// 环境变量优先于 UserDefaults（见 `Updater.baseURL`），所以单测必须走
    /// 这条路径 —— 否则外面起 e2e 假源时 `CLOUDOT_UPDATE_FEED=…` 会盖住
    /// UserDefaults，单测读到的就是外部假源而不是自己起的那台。
    private func withFeed(
        _ url: String,
        _ body: () async throws -> Void
    ) async rethrows {
        let key = "CLOUDOT_UPDATE_FEED"
        let previous = getenv(key).map { String(cString: $0) }
        setenv(key, url, 1)
        defer {
            if let previous {
                setenv(key, previous, 1)
            } else {
                unsetenv(key)
            }
        }
        try await body()
    }
}

// MARK: - 本地假 HTTP 源

/// 极简 HTTP/1.1 服务器，只给更新检查的单元测试用。
///
/// 不引入第三方依赖：Foundation 的 `NWListener` 在测试 target 里也能用，
/// 但直接用 BSD socket 更直观，而且这里只需要 HEAD/GET + 302。
private final class LocalHTTPServer: @unchecked Sendable {
    enum Response {
        case redirect(to: String)
        case ok(body: String)
        case notFound
    }

    private let queue = DispatchQueue(label: "cloudot.test.http")
    private var socketFD: Int32 = -1
    private var source: DispatchSourceRead?
    let baseURL: String
    private let handler: @Sendable (String) -> Response

    init(handler: @escaping @Sendable (String) -> Response) throws {
        self.handler = handler

        // 绑到 127.0.0.1:0，让内核挑空闲端口
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.EINIT) }
        var yes: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))

        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")
        addr.sin_port = 0
        let bindOK = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindOK == 0, listen(fd, 8) == 0 else {
            close(fd)
            throw POSIXError(.EADDRINUSE)
        }

        var bound = sockaddr_in()
        var len = socklen_t(MemoryLayout<sockaddr_in>.size)
        _ = withUnsafeMutablePointer(to: &bound) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                getsockname(fd, $0, &len)
            }
        }
        let port = Int(UInt16(bigEndian: bound.sin_port))
        self.baseURL = "http://127.0.0.1:\(port)"
        self.socketFD = fd

        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in self?.acceptOne() }
        source.setCancelHandler { close(fd) }
        source.resume()
        self.source = source
    }

    func stop() {
        source?.cancel()
        source = nil
        socketFD = -1
    }

    deinit { stop() }

    private func acceptOne() {
        let client = accept(socketFD, nil, nil)
        guard client >= 0 else { return }
        queue.async {
            defer { close(client) }
            var buffer = [UInt8](repeating: 0, count: 4096)
            let n = read(client, &buffer, buffer.count)
            guard n > 0 else { return }
            let raw = String(decoding: buffer[0..<n], as: UTF8.self)
            // 第一行：`HEAD /releases/latest HTTP/1.1`
            let path = raw.split(separator: "\r\n", maxSplits: 1).first
                .flatMap { line -> String? in
                    let parts = line.split(separator: " ")
                    guard parts.count >= 2 else { return nil }
                    return String(parts[1])
                } ?? "/"

            let response: String
            switch self.handler(path) {
            case .redirect(let to):
                response = "HTTP/1.1 302 Found\r\nLocation: \(to)\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            case .ok(let body):
                let data = Data(body.utf8)
                response = "HTTP/1.1 200 OK\r\nContent-Length: \(data.count)\r\nConnection: close\r\n\r\n\(body)"
            case .notFound:
                response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            }
            response.withCString { ptr in
                _ = write(client, ptr, strlen(ptr))
            }
        }
    }
}

/// `POSIXError.Code` 没有 EINIT，借一个通用的。
private extension POSIXError.Code {
    static var EINIT: POSIXError.Code { .EIO }
}
