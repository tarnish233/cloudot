import AppKit
import ImageIO
import Foundation
import XCTest

@testable import Cloudot

/// Swift 侧的 Codable 与 Rust 侧的 `--json` 契约必须一致。
///
/// fixture 全是从**真实 CLI** 抓下来的输出（见仓库 README 里的重抓命令），
/// 不是手写的 —— 手写 fixture 只能验证「我以为的格式」，验证不了实际格式。
final class ContractTests: XCTestCase {
    private let decoder = JSONDecoder()

    private func fixture(_ name: String) throws -> Data {
        let url = try XCTUnwrap(
            Bundle.module.url(forResource: "Fixtures/\(name)", withExtension: "json"),
            "找不到 fixture \(name).json"
        )
        return try Data(contentsOf: url)
    }

    private func decodeOK<R: Decodable>(_ name: String, _ type: R.Type) throws -> R {
        let envelope = try decoder.decode(Envelope<R>.self, from: try fixture(name))
        XCTAssertTrue(envelope.ok, "\(name) 应该是成功信封")
        return envelope.result
    }

    // MARK: - 成功信封

    func testStatusDecodes() throws {
        let status = try decodeOK("status", Status.self)
        XCTAssertFalse(status.device.isEmpty)
        XCTAssertTrue(status.git.repo)
        XCTAssertEqual(status.apps.first?.id, "ghostty")
        XCTAssertEqual(status.apps.first?.files.first?.state, .linked)
        XCTAssertTrue(status.healthy)
        XCTAssertTrue(status.orphans.isEmpty)
    }

    func testSandboxStatusDecodes() throws {
        // e2e 沙盒跑完后的状态，结构与真实环境不同（可能没纳管、有可用应用）
        let status = try decodeOK("status-sandbox", Status.self)
        XCTAssertFalse(status.root.isEmpty)
    }

    func testDoctorDecodes() throws {
        let report = try decodeOK("doctor", DoctorReport.self)
        XCTAssertFalse(report.checks.isEmpty)
        // 每个检查项都要能解出 level，否则界面排序会不对
        XCTAssertTrue(report.checks.allSatisfy { [.ok, .warn, .error].contains($0.level) })
        XCTAssertTrue(report.checks.contains { $0.name == "secrets" })
    }

    func testAppsDecodes() throws {
        let apps = try decodeOK("apps", [AppListing].self)
        XCTAssertEqual(apps.first?.id, "ghostty")
        XCTAssertTrue(apps.first?.managed ?? false)
    }

    func testBackupsDecodes() throws {
        let set = try decodeOK("backups", BackupSet.self)
        // snake_case 的字段名最容易写错，明确验一下
        XCTAssertEqual(set.totalFiles, set.entries.reduce(0) { $0 + $1.files })
        XCTAssertEqual(set.totalBytes, set.entries.reduce(0) { $0 + $1.bytes })
    }

    func testSyncDecodes() throws {
        let result = try decodeOK("sync", SyncResult.self)
        XCTAssertNotNil(result.remote)
        XCTAssertTrue([.skipped, .upToDate, .updated].contains(result.pull))
    }

    func testApplyDecodes() throws {
        let result = try decodeOK("apply", ApplyResult.self)
        XCTAssertTrue(result.items.allSatisfy { !$0.target.isEmpty })
    }

    // MARK: - 错误信封

    func testNotInitializedError() throws {
        let envelope = try decoder.decode(
            Envelope<ErrorResult>.self,
            from: try fixture("error-not-initialized")
        )
        XCTAssertFalse(envelope.ok)
        XCTAssertEqual(envelope.result.kind, .notInitialized)
        XCTAssertEqual(envelope.schema, "cloudot.error/v1")
        XCTAssertFalse(envelope.result.summary.isEmpty)
    }

    func testUnknownAppError() throws {
        let envelope = try decoder.decode(
            Envelope<ErrorResult>.self,
            from: try fixture("error-unknown-app")
        )
        XCTAssertEqual(envelope.result.kind, .unknownApp)
    }

    /// CLI 以后加新的错误分类时，旧版界面不能因为解不出来就整个崩掉 ——
    /// 错误分类只影响引导文案，降级成 `.other` 仍然能把 message 显示出来。
    func testUnknownErrorKindFallsBackToOther() throws {
        let json = #"""
        {"schema":"cloudot.error/v1","ok":false,
         "result":{"kind":"some_future_kind","summary":"未来的错误","message":"细节"}}
        """#
        let envelope = try decoder.decode(
            Envelope<ErrorResult>.self,
            from: Data(json.utf8)
        )
        XCTAssertEqual(envelope.result.kind, .other)
        XCTAssertEqual(envelope.result.message, "细节")
    }

    /// 与上面相反：链接状态**故意**严格解码。
    /// 新增一种状态意味着可能有新的损坏形态，宁可整体解码失败让用户看到
    /// 「命令输出异常」，也不要把它静默降级成一个看起来正常的值。
    func testUnknownLinkStateFailsLoudly() {
        let json = #"""
        {"schema":"cloudot.status/v1","ok":true,"result":{
          "device":"d","root":"/r","healthy":true,
          "git":{"repo":true,"branch":"main","head":"abc","remote":null,
                 "dirty":[],"ahead":0,"behind":0},
          "apps":[{"id":"x","name":"X","adopted_by":"d","ok":true,
                   "files":[{"target":"~/a","store":"files/a","state":"brand_new_state"}]}],
          "available":[],"orphans":[]}}
        """#
        XCTAssertThrowsError(
            try decoder.decode(Envelope<Status>.self, from: Data(json.utf8))
        )
    }

    // MARK: - 派生逻辑

    func testOrphanStatusDrivesUnhealthy() throws {
        let json = #"""
        {"schema":"cloudot.status/v1","ok":true,"result":{
          "device":"d","root":"/r","healthy":false,
          "git":{"repo":true,"branch":"main","head":"abc","remote":null,
                 "dirty":[],"ahead":0,"behind":0},
          "apps":[],"available":[],
          "orphans":[{"app":"ghostty","target":"~/.config/ghostty/config",
                      "store":"files/.config/ghostty/config","kind":"dangling"}]}}
        """#
        let status = try decoder.decode(Envelope<Status>.self, from: Data(json.utf8)).result
        XCTAssertFalse(status.healthy)
        XCTAssertEqual(status.orphans.first?.kind, .dangling)
    }

    func testGitPendingWorkDetection() throws {
        let status = try decodeOK("status", Status.self)
        XCTAssertEqual(
            status.git.hasPendingWork,
            !status.git.dirty.isEmpty || (status.git.ahead ?? 0) > 0 || (status.git.behind ?? 0) > 0
        )
    }

    /// 体检项按严重度排序，否则关键项会被一堆 ✓ 埋掉。
    func testLevelOrdering() {
        XCTAssertTrue(Level.error > Level.warn)
        XCTAssertTrue(Level.warn > Level.ok)
        XCTAssertEqual([Level.ok, .error, .warn].sorted { $0 > $1 }, [.error, .warn, .ok])
    }

    func testOnlyMissingIsFixableByPlainApply() {
        XCTAssertTrue(LinkState.missing.fixableByApply)
        // 本地是实体文件时内容可能比 store 新，不带 --force 的 apply 会拒绝
        XCTAssertFalse(LinkState.replacedByFile.fixableByApply)
        XCTAssertFalse(LinkState.foreignSymlink.fixableByApply)
        XCTAssertFalse(LinkState.storeMissing.fixableByApply)
    }

    func testStampFormatting() {
        XCTAssertEqual(Format.stamp("20260801-120750"), "2026-08-01 12:07")
        // 解不出来就原样显示，不要崩也不要显示错的时间
        XCTAssertEqual(Format.stamp("手动备份"), "手动备份")
    }

    /// 需要在终端收尾的错误要给出具体命令；其余不要画蛇添足。
    func testTerminalHintsOnlyForSwitchGatedErrors() {
        for kind in [ErrorKind.needsForce, .secretsDetected, .pullConflict, .notInitialized] {
            let banner = Banner(tone: .failure, title: "t", detail: nil, kind: kind, subject: nil)
            XCTAssertNotNil(banner.terminalHint, "\(kind) 应该给出终端命令")
        }
        for kind in [ErrorKind.locked, .unknownApp, .notAdopted, .other] {
            let banner = Banner(tone: .failure, title: "t", detail: nil, kind: kind, subject: nil)
            XCTAssertNil(banner.terminalHint, "\(kind) 不需要终端命令")
        }
    }

    /// 引导命令要能直接复制运行，不能留 `<应用>` 这种占位符让用户自己换。
    ///
    /// GUI 刻意不提供 `--allow-secrets` / `--force`，所以「去终端怎么敲」是这条路
    /// 唯一的出口 —— 后端明明知道是哪个应用，界面就不该把这个信息丢掉。
    func testSecretsHintNamesTheActualApp() {
        let named = Banner(
            tone: .failure, title: "扫到疑似凭据", detail: nil,
            kind: .secretsDetected, subject: "gitpic"
        )
        let hint = named.terminalHint ?? ""
        XCTAssertTrue(hint.contains("gitpic"), "引导命令里应该出现具体的应用名：\(hint)")
        XCTAssertFalse(hint.contains("<应用>"), "不该留占位符：\(hint)")

        // 不知道对象时退回占位符，总比给出一条错命令好
        let anonymous = Banner(
            tone: .failure, title: "扫到疑似凭据", detail: nil,
            kind: .secretsDetected, subject: nil
        )
        XCTAssertTrue(anonymous.terminalHint?.contains("<应用>") ?? false)
    }
}

/// 代码绘制的小机器人动效。
@MainActor
final class RobotAnimatorTests: XCTestCase {
    private let allStates: [AppModel.IconState] =
        [.healthy, .pending, .refreshing, .syncing, .broken, .unavailable]

    private func png(_ image: NSImage) -> Data? {
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff) else { return nil }
        return rep.representation(using: .png, properties: [:])
    }

    // MARK: - 序列结构

    func testEveryStateHasFrames() {
        for state in allStates {
            XCTAssertFalse(RobotAnimator.resting(for: state).frames.isEmpty,
                           "\(state) 没有帧，菜单栏会空白")
        }
    }

    func testLoopingIsCorrect() {
        XCTAssertTrue(RobotAnimator.idle.loops)
        XCTAssertTrue(RobotAnimator.pending.loops)
        XCTAssertTrue(RobotAnimator.refreshing.loops)
        XCTAssertTrue(RobotAnimator.working.loops)
        XCTAssertFalse(RobotAnimator.broken.loops)
        XCTAssertFalse(RobotAnimator.offline.loops)
        XCTAssertFalse(RobotAnimator.success.loops, "一次性动效不能循环")
        XCTAssertFalse(RobotAnimator.failure.loops, "一次性动效不能循环")
    }

    func testRefreshHasAVisibleScanAnimation() throws {
        let frames = RobotAnimator.refreshing.frames
        let datas = try frames.map { try XCTUnwrap(png($0)) }
        XCTAssertGreaterThan(Set(datas).count, 6, "刷新时眼睛没有真正扫描")
        XCTAssertEqual(RobotAnimator.refreshing.interval, 0.08, accuracy: 0.001)
    }

    /// 尺寸恒定，否则菜单栏项本身会跟着动画跳动。
    func testAllFramesShareCanvasSize() {
        let sequences = [
            RobotAnimator.idle, RobotAnimator.pending, RobotAnimator.refreshing,
            RobotAnimator.working, RobotAnimator.broken, RobotAnimator.offline,
            RobotAnimator.success, RobotAnimator.failure,
        ]
        for seq in sequences {
            for frame in seq.frames {
                XCTAssertEqual(frame.size, RobotAnimator.canvasSize)
            }
        }
        XCTAssertGreaterThan(RobotAnimator.canvasSize.width, RobotAnimator.canvasSize.height)
        XCTAssertEqual(RobotAnimator.canvasSize.height, RobotAnimator.barHeight, accuracy: 0.01)
    }

    /// template 让 AppKit 在深色菜单栏使用白色，在浅色菜单栏自动反转。
    func testEveryFrameUsesSystemTemplateRendering() {
        let sequences = [
            RobotAnimator.idle, RobotAnimator.pending, RobotAnimator.refreshing,
            RobotAnimator.working, RobotAnimator.broken, RobotAnimator.offline,
            RobotAnimator.success, RobotAnimator.failure,
        ]
        for frame in sequences.flatMap(\.frames) {
            XCTAssertTrue(frame.isTemplate)
        }
    }

    func testBrokenDiffersFromIdle() throws {
        XCTAssertNotEqual(try XCTUnwrap(png(RobotAnimator.broken.frames[0])),
                          try XCTUnwrap(png(RobotAnimator.idle.frames[0])))
    }

    func testOfflineDiffersFromIdle() throws {
        XCTAssertNotEqual(try XCTUnwrap(png(RobotAnimator.offline.frames[0])),
                          try XCTUnwrap(png(RobotAnimator.idle.frames[0])))
    }

    func testSuccessHasMotionAndReturnsToNeutral() throws {
        let frames = RobotAnimator.success.frames
        let datas = try frames.map { try XCTUnwrap(png($0)) }
        XCTAssertGreaterThan(Set(datas).count, 4, "成功动画没有动起来")
        XCTAssertEqual(try XCTUnwrap(png(frames.last!)),
                       try XCTUnwrap(png(RobotAnimator.idle.frames[0])))
    }

    func testFailureHasMotionAndReturnsToNeutral() throws {
        let frames = RobotAnimator.failure.frames
        let datas = try frames.map { try XCTUnwrap(png($0)) }
        XCTAssertGreaterThan(Set(datas).count, 5, "失败动画没有摇头")
        XCTAssertEqual(try XCTUnwrap(png(frames.last!)),
                       try XCTUnwrap(png(RobotAnimator.idle.frames[0])))
    }

    func testPulseMapsToOneShot() {
        XCTAssertEqual(RobotAnimator.oneShot(for: .success).frames.count,
                       RobotAnimator.success.frames.count)
        XCTAssertEqual(RobotAnimator.oneShot(for: .failure).frames.count,
                       RobotAnimator.failure.frames.count)
    }

    func testSequencesAreMemoized() {
        for (a, b) in zip(RobotAnimator.refreshing.frames, RobotAnimator.refreshing.frames) {
            XCTAssertTrue(a === b, "帧没缓存，动画路径上仍在重新合成")
        }
    }

    /// 导出动图预览，方便肉眼验收动效。默认不跑 ——
    /// 设 CLOUDOT_WRITE_PREVIEW=1 才写文件。
    func testWritePreviewGIFs() throws {
        try XCTSkipIf(ProcessInfo.processInfo.environment["CLOUDOT_WRITE_PREVIEW"] == nil)
        let named: [(String, RobotAnimationSequence)] = [
            ("1-正常", RobotAnimator.idle),
            ("2-待同步", RobotAnimator.pending),
            ("3-刷新", RobotAnimator.refreshing),
            ("4-同步中", RobotAnimator.working),
            ("5-同步成功", RobotAnimator.success),
            ("6-同步失败", RobotAnimator.failure),
            ("7-有损坏", RobotAnimator.broken),
            ("8-找不到CLI", RobotAnimator.offline),
        ]
        for (name, seq) in named {
            try writeGIF(seq, to: "/tmp/cloudot-robot/\(name).gif")
        }
        print("PREVIEW-WRITTEN /tmp/cloudot-robot")
    }

    /// 放大 8 倍写成循环动图，方便肉眼验收 18pt 里的细节。
    private func writeGIF(_ seq: RobotAnimationSequence, to path: String) throws {
        try FileManager.default.createDirectory(
            atPath: (path as NSString).deletingLastPathComponent,
            withIntermediateDirectories: true)
        let scale: CGFloat = 8
        let size = NSSize(width: RobotAnimator.canvasSize.width * scale,
                          height: RobotAnimator.canvasSize.height * scale)
        let url = URL(filePath: path)
        guard let dest = CGImageDestinationCreateWithURL(
            url as CFURL, "com.compuserve.gif" as CFString, seq.frames.count, nil) else { return }
        CGImageDestinationSetProperties(dest, [
            kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0],
        ] as CFDictionary)
        for frame in seq.frames {
            let big = NSImage(size: size)
            big.lockFocus()
            NSColor(white: 0.13, alpha: 1).setFill()
            NSRect(origin: .zero, size: size).fill()
            NSGraphicsContext.current?.imageInterpolation = .high
            frame.draw(in: NSRect(origin: .zero, size: size))
            big.unlockFocus()
            guard let cg = big.cgImage(forProposedRect: nil, context: nil, hints: nil)
            else { continue }
            CGImageDestinationAddImage(dest, cg, [
                kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: seq.interval],
            ] as CFDictionary)
        }
        CGImageDestinationFinalize(dest)
    }
}


/// 自动刷新的节流策略。
///
/// 这些是「看不见」的行为 —— 间隔被人不小心改回 20 秒、或者后台刷新又开始驱动
/// 菜单栏动画，界面上都不会报错，只会变得费电又晃眼。所以用测试钉住。
@MainActor
final class RefreshPolicyTests: XCTestCase {
    /// 轮询间隔要足够长。
    ///
    /// `status` / `doctor` 读的是本地缓存的 `@{upstream}` ref，**不 fetch**，所以
    /// 轮询发现不了远端的新提交；它唯一能新发现的是「本机改了配置」，而那件事
    /// 用户自己清楚。真正保证数据新鲜的是打开面板时的那次 status 刷新。
    func testPollingIsLazy() {
        XCTAssertGreaterThanOrEqual(
            AppModel.refreshInterval.timeInterval, 3600,
            "轮询发现不了远端改动，没必要频繁跑"
        )
    }

    /// `Duration` 换算成秒不能出错 —— 上面两条断言全靠它。
    func testDurationConversion() {
        XCTAssertEqual(Duration.seconds(20).timeInterval, 20, accuracy: 0.001)
        XCTAssertEqual(Duration.seconds(24 * 60 * 60).timeInterval, 86_400, accuracy: 0.001)
        XCTAssertEqual(Duration.milliseconds(1500).timeInterval, 1.5, accuracy: 0.001)
    }

    /// 后台刷新不进 `.refreshing` 状态，菜单栏因此不会播扫眼动画。
    ///
    /// 原来的问题不只是「晃」：`refreshing` 是 12 帧 ×80ms = 0.96s，而一次刷新
    /// 只要约 0.17s，动画连一轮都播不完就被打断 —— 看起来是眼睛歪一下又弹回去。
    func testBackgroundRefreshDoesNotAnimate() async {
        let model = AppModel()
        // 找不到 CLI 时状态恒为 unavailable，测不出东西
        try? XCTSkipIf(model.locateError != nil, "本机没装 cloudot")

        await model.refresh()                      // 后台刷新（默认）
        XCTAssertNotEqual(model.iconState, .refreshing,
                          "后台刷新不该驱动菜单栏动画")
    }

    /// `refreshing` 这个状态本身仍然要有动画 —— 用户点刷新时得有反馈。
    func testRefreshingStateStillHasAnimation() {
        let seq = RobotAnimator.resting(for: .refreshing)
        XCTAssertGreaterThan(seq.frames.count, 1, "手动刷新应当有动画反馈")
        XCTAssertTrue(seq.loops, "刷新时长不确定，动画要能循环")
    }
}
