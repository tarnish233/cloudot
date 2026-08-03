import AppKit
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
        // 旧 fixture 没有 initialized 字段时按已初始化处理
        XCTAssertTrue(status.initialized)
    }

    func testSandboxStatusDecodes() throws {
        // e2e 沙盒跑完后的状态，结构与真实环境不同（可能没纳管、有可用应用）
        let status = try decodeOK("status-sandbox", Status.self)
        XCTAssertFalse(status.root.isEmpty)
    }

    /// 未初始化是**成功**信封，不是错误 —— GUI 靠 `initialized: false` 画引导页。
    func testUninitializedStatusDecodes() throws {
        let status = try decodeOK("status-uninitialized", Status.self)
        XCTAssertFalse(status.initialized)
        XCTAssertFalse(status.healthy)
        XCTAssertTrue(status.apps.isEmpty)
        XCTAssertFalse(status.git.repo)
    }

    func testDoctorDecodes() throws {
        let report = try decodeOK("doctor", DoctorReport.self)
        XCTAssertFalse(report.checks.isEmpty)
        // 每个检查项都要能解出 level，否则界面排序会不对
        XCTAssertTrue(report.checks.allSatisfy { [.ok, .warn, .error].contains($0.level) })
        XCTAssertTrue(report.checks.contains { $0.name == "secrets" })
    }

    func testDoctorChecksAreCategorized() throws {
        let report = try decodeOK("doctor", DoctorReport.self)
        let categorized = DoctorCategory.allCases.flatMap { $0.checks(in: report) }

        XCTAssertEqual(Set(categorized.map(\.id)), Set(report.checks.map(\.id)))
        XCTAssertEqual(
            DoctorCategory.category(for: try XCTUnwrap(report.checks.first { $0.name == "sync-state" })),
            .synchronization
        )
        XCTAssertEqual(
            DoctorCategory.category(for: try XCTUnwrap(report.checks.first { $0.name.hasPrefix("link:") })),
            .files
        )
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
        // 写路径的 not_initialized 没有 conflict 字段
        XCTAssertNil(envelope.result.conflict)
    }

    func testUnknownAppError() throws {
        let envelope = try decoder.decode(
            Envelope<ErrorResult>.self,
            from: try fixture("error-unknown-app")
        )
        XCTAssertEqual(envelope.result.kind, .unknownApp)
    }

    /// 拉取冲突：错误信封带结构化 conflict（文件列表 + diff），供 GUI 选边。
    func testPullConflictErrorCarriesDiff() throws {
        let envelope = try decoder.decode(
            Envelope<ErrorResult>.self,
            from: try fixture("error-pull-conflict")
        )
        XCTAssertFalse(envelope.ok)
        XCTAssertEqual(envelope.result.kind, .pullConflict)
        let report = try XCTUnwrap(envelope.result.conflict)
        XCTAssertEqual(report.branch, "main")
        XCTAssertEqual(report.remoteRef, "origin/main")
        XCTAssertFalse(report.files.isEmpty)
        let file = try XCTUnwrap(report.files.first)
        XCTAssertEqual(file.path, "files/.config/ghostty/config")
        XCTAssertTrue(file.diff.contains("theme = solarized") || file.diff.contains("theme = light"))
        XCTAssertFalse(file.truncated)
    }

    /// 普通错误信封没有 conflict 字段时必须解成 nil，不能整包失败。
    func testErrorWithoutConflictFieldDecodes() throws {
        let json = #"""
        {"schema":"cloudot.error/v1","ok":false,
         "result":{"kind":"locked","summary":"被锁","message":"另一进程在跑"}}
        """#
        let envelope = try decoder.decode(
            Envelope<ErrorResult>.self,
            from: Data(json.utf8)
        )
        XCTAssertEqual(envelope.result.kind, .locked)
        XCTAssertNil(envelope.result.conflict)
    }

    func testInitAndResolveResultsDecode() throws {
        let initJSON = #"""
        {"schema":"cloudot.init/v1","ok":true,"result":{
          "root":"/tmp/x","device":"mbp","remote":"git@example.com:a/b.git",
          "cloned":true,"already":false,"apps_in_store":2}}
        """#
        let initOut = try decoder.decode(Envelope<InitResult>.self, from: Data(initJSON.utf8)).result
        XCTAssertEqual(initOut.device, "mbp")
        XCTAssertEqual(initOut.appsInStore, 2)
        XCTAssertTrue(initOut.cloned)

        let resolveJSON = #"""
        {"schema":"cloudot.resolve/v1","ok":true,"result":{
          "side":"theirs","target":"origin/main","head":"abc1234",
          "applied":{"items":[],"healed":[]}}}
        """#
        let resolveOut = try decoder.decode(Envelope<ResolveResult>.self, from: Data(resolveJSON.utf8)).result
        XCTAssertEqual(resolveOut.side, .theirs)
        XCTAssertEqual(resolveOut.target, "origin/main")
        XCTAssertNotNil(resolveOut.applied)
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

/// 菜单栏静态图标。
///
/// 这些是**编译器管不到**的不变量：`switch` 已经保证每个状态都有符号名，但符号名写错、
/// 或者某个符号在部署目标上根本不存在，都只在运行时表现成「菜单栏空了一块」——
/// 一个不报错、只会消失的故障。
@MainActor
final class MenuBarIconTests: XCTestCase {

    /// 遍历 allCases 而不是手写数组：以后加第七个状态，这条会自动覆盖到。
    func testEveryStateResolvesToAnImage() {
        for state in AppModel.IconState.allCases {
            XCTAssertNotNil(MenuBarIcon.image(for: state),
                            "\(state) 的符号 \(state.symbol) 在本机解析不出来")
        }
    }

    func testEveryPulseResolvesToAnImage() {
        for kind in AppModel.IconPulse.Kind.allCases {
            XCTAssertNotNil(MenuBarIcon.image(for: kind),
                            "\(kind) 的符号 \(kind.symbol) 在本机解析不出来")
        }
    }

    /// template 让 AppKit 在深色菜单栏用白色、浅色菜单栏自动反转。
    /// `withSymbolConfiguration` 返回的是新实例，`isTemplate` 特别容易在那一步丢掉。
    func testEveryImageIsTemplate() throws {
        for state in AppModel.IconState.allCases {
            XCTAssertTrue(try XCTUnwrap(MenuBarIcon.image(for: state)).isTemplate, "\(state)")
        }
        for kind in AppModel.IconPulse.Kind.allCases {
            XCTAssertTrue(try XCTUnwrap(MenuBarIcon.image(for: kind)).isTemplate, "\(kind)")
        }
    }

    /// squareLength 把菜单栏项固定成正方形，比它宽的符号会被裁掉 ——
    /// 这批里 `icloud.slash` 最宽，字号一往上调就先撞它。
    func testNoSymbolOverflowsTheSquareItem() throws {
        let limit = NSStatusBar.system.thickness
        try XCTSkipIf(limit <= 0, "拿不到菜单栏高度（无窗口服务器）")
        for state in AppModel.IconState.allCases {
            let size = try XCTUnwrap(MenuBarIcon.image(for: state)).size
            XCTAssertLessThanOrEqual(size.width, limit, "\(state)（\(state.symbol)）会被裁掉")
            XCTAssertLessThanOrEqual(size.height, limit, "\(state)（\(state.symbol)）比菜单栏还高")
        }
    }

    /// 「有改动等着你同步」必须和别的状态长得不一样 ——
    /// 这是菜单栏唯一真正要传达的区别：**要不要我动手**。
    func testPendingIsNotConfusableWithAnythingElse() {
        let others = AppModel.IconState.allCases
            .filter { $0 != .pending }
            .map(\.symbol)
        XCTAssertFalse(others.contains(AppModel.IconState.pending.symbol),
                       "待同步撞了别的状态：\(AppModel.IconState.pending.symbol)")
    }

    /// 菜单栏只在**需要你做事**时才改形状。
    ///
    /// healthy / refreshing / syncing 刻意共用常态那个同步环：静态图标区分不了
    /// 「在跑」，而那件事由面板里的 ProgressView 和 tooltip 负责说。把「刻意」
    /// 断言下来，免得以后有人当成漏了去「修」。
    func testInFlightSharesTheRestingSymbol() {
        XCTAssertEqual(AppModel.IconState.refreshing.symbol, AppModel.IconState.healthy.symbol)
        XCTAssertEqual(AppModel.IconState.syncing.symbol, AppModel.IconState.healthy.symbol)
    }

    /// 需要你处理的三种状态之间不能撞脸 —— 它们对应三种不同的处置。
    func testActionableStatesLookDistinct() {
        let actionable: [AppModel.IconState] = [.healthy, .pending, .broken, .unavailable]
        XCTAssertEqual(Set(actionable.map(\.symbol)).count, actionable.count,
                       "有状态撞了图标：\(actionable.map(\.symbol))")
    }

    /// 常态图标要和 App 图标同源。`Icon/make-icon.swift` 画的就是这个符号，
    /// 改任何一边都得同时改另一边，否则菜单栏和 Finder 里会是两个东西。
    func testRestingSymbolMatchesTheAppIcon() {
        XCTAssertEqual(AppModel.IconState.healthy.symbol, "arrow.triangle.2.circlepath")
    }

    /// 结果反馈不能和任何常驻姿态撞脸，否则那 0.7 秒看不出发生过事。
    func testPulseSymbolsDifferFromEveryRestingSymbol() {
        let resting = Set(AppModel.IconState.allCases.map(\.symbol))
        for kind in AppModel.IconPulse.Kind.allCases {
            XCTAssertFalse(resting.contains(kind.symbol), "\(kind) 和某个常驻状态同图标")
        }
    }
}


/// 自动刷新的节流策略。
///
/// 这些是「看不见」的行为 —— 间隔被人不小心改回 20 秒、或者后台刷新又开始驱动
/// 菜单栏图标，界面上都不会报错，只会变得费电又晃眼。所以用测试钉住。
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

    /// 后台刷新不进 `.refreshing` 状态。
    ///
    /// 这条现在钉的是**状态机**，不是图标 —— `refreshing` 和 `healthy` 已经共用
    /// 同一个符号，所以图标本来就不会变。但状态本身还是要分清：`isBusy` 驱动着
    /// 面板里的 ProgressView 和「刷新/同步」按钮的禁用，每 20 秒闪一下同样烦人。
    func testBackgroundRefreshDoesNotEnterRefreshingState() async {
        let model = AppModel()
        // 找不到 CLI 时状态恒为 unavailable，测不出东西
        try? XCTSkipIf(model.locateError != nil, "本机没装 cloudot")

        await model.refresh()                      // 后台刷新（默认）
        XCTAssertNotEqual(model.iconState, .refreshing,
                          "后台刷新不该进 refreshing 状态")
    }
}
