import Foundation
import XCTest

@testable import Cloudot

/// FSEvents 监视器。
///
/// 这些测试真的动文件、真的等事件 —— 监视这种东西没法靠读代码验证，
/// 参数写错（漏 FileEvents 标志、latency 给太大）都能正常编译，只是不工作。
final class ConfigWatcherTests: XCTestCase {
    private var root: URL!

    override func setUp() {
        super.setUp()
        root = URL(filePath: "/tmp/cloudot-watcher-test-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: root)
        super.tearDown()
    }

    /// 等一个变化通知，超时就失败。
    ///
    /// FSEvents 的 latency 是 1 秒，加上文件系统本身的延迟，给 6 秒余量 ——
    /// 太短会变成随机失败的测试，那比没有测试更糟。
    private func expectChange(
        timeout: TimeInterval = 6,
        setUp: (ConfigWatcher) -> Void,
        trigger: () throws -> Void
    ) rethrows -> Bool {
        let fired = expectation(description: "收到文件变化通知")
        fired.assertForOverFulfill = false      // 一次保存可能报多个事件

        let watcher = ConfigWatcher { fired.fulfill() }
        setUp(watcher)
        defer { watcher.stop() }

        // 等 stream 真正跑起来再动文件，否则可能在订阅生效前就改完了
        Thread.sleep(forTimeInterval: 0.6)
        try trigger()

        return XCTWaiter.wait(for: [fired], timeout: timeout) == .completed
    }

    /// 场景一：通过软链写入（用户改配置最常见的方式）。
    ///
    /// 实测这种情况**只在 store 那边报事件**，`~/.config` 一侧一个都没有 ——
    /// 所以监视范围必须包含 store，否则最常见的改配置根本发现不了。
    func testWritingThroughSymlinkIsDetectedInStore() throws {
        let store = root.appending(path: "store/files/.config/app")
        let cfg = root.appending(path: ".config/app")
        try FileManager.default.createDirectory(at: store, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: cfg, withIntermediateDirectories: true)
        let target = store.appending(path: "config")
        let link = cfg.appending(path: "config")
        try "a = 1\n".write(to: target, atomically: true, encoding: .utf8)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: target)

        let detected = try expectChange { watcher in
            watcher.start(watching: [store])          // 只看 store
        } trigger: {
            let handle = try XCTUnwrap(FileHandle(forWritingAtPath: link.path()))
            handle.seekToEndOfFile()
            handle.write(Data("b = 2\n".utf8))
            handle.closeFile()
        }
        XCTAssertTrue(detected, "通过软链改配置，store 侧应当收到事件")
    }

    /// 场景二：替换写入顶掉软链（karabiner GUI 保存就是这样）。
    ///
    /// 这种情况**只在 `~/.config` 那边报事件**。这是最需要报警的状态
    /// （软链没了，之后的改动不再进 store），所以监视范围必须也包含配置目录。
    func testReplaceWriteClobberingSymlinkIsDetected() throws {
        let cfg = root.appending(path: ".config/app")
        let store = root.appending(path: "store/files/.config/app")
        try FileManager.default.createDirectory(at: cfg, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: store, withIntermediateDirectories: true)
        let target = store.appending(path: "config")
        let link = cfg.appending(path: "config")
        try "a = 1\n".write(to: target, atomically: true, encoding: .utf8)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: target)

        let detected = try expectChange { watcher in
            watcher.start(watching: [cfg])            // 只看配置目录
        } trigger: {
            let tmp = cfg.appending(path: ".tmp")
            try "clobbered\n".write(to: tmp, atomically: false, encoding: .utf8)
            rename(tmp.path(), link.path())           // rename 会顶掉软链
        }
        XCTAssertTrue(detected, "软链被替换写入顶掉时，配置目录侧应当收到事件")
    }

    /// 场景三：store 里的文件直接被改（`sync` 拉下远端新内容）。
    func testDirectStoreWriteIsDetected() throws {
        let store = root.appending(path: "store/files")
        try FileManager.default.createDirectory(at: store, withIntermediateDirectories: true)
        let file = store.appending(path: "config")
        try "old\n".write(to: file, atomically: true, encoding: .utf8)

        let detected = try expectChange { watcher in
            watcher.start(watching: [store])
        } trigger: {
            try "from-remote\n".write(to: file, atomically: true, encoding: .utf8)
        }
        XCTAssertTrue(detected, "store 文件被改时应当收到事件")
    }

    /// 不存在的目录要安静跳过，不能崩也不能让整个监视失效。
    ///
    /// 用户可能纳管了一个应用然后手动删掉整个配置目录。
    func testMissingDirectoriesAreSkipped() throws {
        let real = root.appending(path: "real")
        try FileManager.default.createDirectory(at: real, withIntermediateDirectories: true)

        let detected = try expectChange { watcher in
            watcher.start(watching: [
                root.appending(path: "does-not-exist"),
                real,
            ])
        } trigger: {
            try "x\n".write(to: real.appending(path: "f"), atomically: true, encoding: .utf8)
        }
        XCTAssertTrue(detected, "有一个目录不存在，不该影响其余目录的监视")
    }

    /// 全部目录都不存在时不该建 stream —— 建了也收不到东西，白占资源。
    func testNoStreamWhenNothingExists() {
        let watcher = ConfigWatcher {}
        watcher.start(watching: [root.appending(path: "nope")])
        XCTAssertFalse(watcher.isWatching)
        watcher.stop()
    }

    /// `stop` 之后不再有通知；重复 `start` 不泄漏 stream。
    func testStopAndRestart() throws {
        let dir = root.appending(path: "d")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        let watcher = ConfigWatcher {}
        watcher.start(watching: [dir])
        XCTAssertTrue(watcher.isWatching)
        watcher.start(watching: [dir])          // 重复 start：内部先 stop 再建
        XCTAssertTrue(watcher.isWatching)
        watcher.stop()
        XCTAssertFalse(watcher.isWatching)
        watcher.stop()                          // 重复 stop 安全
        XCTAssertFalse(watcher.isWatching)
    }
}
