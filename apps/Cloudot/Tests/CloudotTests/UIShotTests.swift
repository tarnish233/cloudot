// 界面自查工具，不是断言测试 —— 默认跳过，`CLOUDOT_UI_SHOT=1` 才跑。
//
// 界面改动很难自动验证：布局错了照样编译通过、测试全绿。这里把视图挂进屏幕外的
// 真实 NSWindow，跑一会儿 run loop 再 cacheDisplay 成 PNG，用来肉眼查排版。
// 已经靠它抓到过 `LabeledContent` 在 GroupBox 里居中、`chart.bar` 在小字号糊成一团。
//
// 用法：
//   CLOUDOT_UI_SHOT=1 CLOUDOT_HOME=/tmp/某个沙盒 ./test.sh --filter UIShotTests
//   开 Finder 看 /tmp/uishot/
//
// **必须挂真实窗口**：侧栏是 NSTableView 撑的，脱离窗口不生成行，会渲出一片空白
// 误导判断。另外这只能查排版，**点击是否真的有反应仍然要人工确认**。
import AppKit
import SwiftUI
import XCTest
@testable import Cloudot

@MainActor
final class UIShotTests: XCTestCase {
    /// `nonisolated` 是因为 `setUpWithError` 不在 main actor 上跑，
    /// 引用 `@MainActor` 隔离的静态属性会告警。
    private nonisolated static let outputDirectory = "/tmp/uishot"

    override func setUpWithError() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["CLOUDOT_UI_SHOT"] == "1")
        try FileManager.default.createDirectory(
            atPath: Self.outputDirectory, withIntermediateDirectories: true)
    }

    // MARK: - 截图

    func testCaptureMainWindow() throws {
        // 整窗（含侧栏），看的是三栏比例和标题栏安全区
        let model = AppModel()
        try capture(MainWindow(model: model), size: NSSize(width: 980, height: 700),
                    settle: 9, named: "main")
    }

    func testCaptureEachPane() throws {
        let model = AppModel()
        // 直接渲染 Pane 绕过了 MainWindow.task，得自己触发一次拉取，
        // 否则各页面只截到空态/加载态 —— 空态下看不出真实排版。
        Task { await model.refresh(userInitiated: true) }
        waitUntil(timeout: 15) {
            !model.appList.isEmpty && model.doctor != nil && model.backups != nil
        }
        print("数据就绪：apps=\(model.appList.count) doctor=\(model.doctor != nil) backups=\(model.backups != nil)")

        for pane in Pane.allCases {
            let content = VStack(spacing: 0) {
                MainWindowHeader(pane: pane, model: model)
                Divider()
                switch pane {
                case .overview: OverviewPane(model: model)
                case .apps: AppsPane(model: model)
                case .doctor: DoctorPane(model: model)
                case .backups: BackupsPane(model: model)
                case .about: AboutPane(model: model)
                }
            }
            try capture(content, size: NSSize(width: 760, height: 560),
                        settle: 2.5, named: "pane-\(pane.rawValue)")
        }
    }

    // MARK: - 脚手架

    /// 把视图挂进屏幕外的真实窗口，等布局稳定后存成 PNG。
    private func capture(
        _ view: some View, size: NSSize, settle: TimeInterval, named name: String
    ) throws {
        let host = NSHostingView(
            rootView: view
                .frame(width: size.width, height: size.height, alignment: .topLeading)
                .background(Color(nsColor: .windowBackgroundColor))
        )
        host.frame = NSRect(origin: .zero, size: size)

        let window = NSWindow(contentRect: host.frame, styleMask: [.titled],
                              backing: .buffered, defer: false)
        // 不设这个的话，函数返回时窗口被释放，下一次调用直接段错误。
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.setFrameOrigin(NSPoint(x: -6000, y: -6000))   // 挪到屏幕外，不打扰人
        window.makeKeyAndOrderFront(nil)

        waitUntil(timeout: settle) { false }   // 单纯等布局稳定

        let rep = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        let path = "\(Self.outputDirectory)/\(name).png"
        try XCTUnwrap(rep.representation(using: .png, properties: [:]))
            .write(to: URL(filePath: path))
        print("已写出 \(path)")
    }

    /// 转 run loop 直到条件成立或超时。SwiftUI 的布局和 CLI 的异步拉取都靠它推进。
    private func waitUntil(timeout: TimeInterval, _ done: () -> Bool) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.05))
            if done() { return }
        }
    }
}
