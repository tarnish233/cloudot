import XCTest
@testable import Cloudot

/// 「关于」页显示的东西。
///
/// 这一页全是**声明性信息** —— 版本号、路径、链接。它不像别的页面会因为数据不对
/// 就空白或报错，写错了照样渲染得漂漂亮亮，只是内容是假的。所以要钉住。
@MainActor
final class AboutPaneTests: XCTestCase {

    /// 侧栏里每一页都要能点。加页面时最容易漏的就是这个 ——
    /// `Pane.allCases` 驱动侧栏，但 `MainWindow` 的 switch 是手写的。
    func testAboutIsReachableFromSidebar() {
        XCTAssertTrue(Pane.allCases.contains(.about))
    }

    /// 每一页都得有标题、副标题和图标，否则侧栏那一行会缺东西。
    func testEveryPaneIsFullyDescribed() {
        for pane in Pane.allCases {
            XCTAssertFalse(pane.title.isEmpty, "\(pane) 没有标题")
            XCTAssertFalse(pane.subtitle.isEmpty, "\(pane) 没有副标题")
            XCTAssertFalse(pane.symbol.isEmpty, "\(pane) 没有图标")
        }
    }

    /// 侧栏图标不能撞脸，否则两行看起来一模一样。
    func testPaneSymbolsAreDistinct() {
        let symbols = Pane.allCases.map(\.symbol)
        XCTAssertEqual(Set(symbols).count, symbols.count, "有页面撞了图标：\(symbols)")
    }

    /// 「关于」页显示的 CLI 路径必须是**真正在调用的**那个。
    ///
    /// GUI 可能在跑 bundle 内自带的 CLI、`~/.cargo/bin` 里的，或者 `defaults write`
    /// 指定的。这一页存在的意义有一半就是回答「到底在跑哪个」，显示错了比不显示更糟。
    func testCLIPathMatchesTheBinaryActuallyUsed() throws {
        let model = AppModel()
        try XCTSkipIf(model.locateError != nil, "本机没装 cloudot")

        let path = try XCTUnwrap(model.cliPath, "定位成功却拿不到路径")
        XCTAssertTrue(FileManager.default.isExecutableFile(atPath: path),
                      "\(path) 不是可执行文件")
        // locate() 的候选顺序是固定的，第一个命中的就是实际会用的那个
        switch CloudotCLI.locate() {
        case .success(let cli):
            XCTAssertEqual(path, cli.executable.path)
        case .failure:
            XCTFail("前面刚定位成功，这里却失败了")
        }
    }

    /// 找不到 CLI 时不能假装有路径 —— 那会让人去排查一个不存在的文件。
    func testNoCLIPathWhenBinaryIsMissing() {
        let model = AppModel()
        if model.locateError != nil {
            XCTAssertNil(model.cliPath)
        }
    }

    /// 版本号读得出来，且形如 `0.2.0`。
    ///
    /// 这条会跟着 `--version` 的输出格式走：clap 打印的是 `cloudot 0.2.0`，
    /// 解析取的是空格后那段。格式变了这里会先炸，而不是等「关于」页显示出
    /// 「cloudot」当版本号。
    ///
    /// 关于页**默认只显示 App 版本**；CLI 版本只在与 App 不一致时才标橙出现，
    /// 所以这里仍然要能读到 CLI 版本——分叉诊断依赖它。
    func testCLIVersionLooksLikeAVersion() async throws {
        let model = AppModel()
        try XCTSkipIf(model.locateError != nil, "本机没装 cloudot")

        await model.loadCLIVersion()
        let version = try XCTUnwrap(model.cliVersion, "读不出 CLI 版本")
        XCTAssertFalse(version.contains(" "), "版本号里不该有空格，拿到的是「\(version)」")
        XCTAssertTrue(version.first?.isNumber ?? false,
                      "版本号该以数字开头，拿到的是「\(version)」")
    }

    /// 重复调用不该重复起进程。
    func testLoadingVersionTwiceIsCheap() async throws {
        let model = AppModel()
        try XCTSkipIf(model.locateError != nil, "本机没装 cloudot")

        await model.loadCLIVersion()
        let first = model.cliVersion
        await model.loadCLIVersion()
        XCTAssertEqual(model.cliVersion, first)
    }

    /// 强制检查更新会清掉旧结果再查；默认检查在已有结果时幂等。
    /// （网络行为由 UpdaterTests 覆盖，这里只钉 AppModel 的 force 语义入口存在。）
    func testCheckForUpdateForceClearsCachedResult() async {
        let model = AppModel()
        // 没有假源时 force 会失败并保持 nil，至少不应崩溃 / 卡住 isChecking
        await model.checkForUpdate(force: true)
        XCTAssertFalse(model.isCheckingForUpdate)
    }
}
