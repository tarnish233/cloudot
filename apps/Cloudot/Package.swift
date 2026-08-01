// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Cloudot",
    platforms: [.macOS(.v15)],
    targets: [
        .executableTarget(
            name: "Cloudot",
            path: "Sources/Cloudot",
            swiftSettings: [
                // SwiftUI 的 App 入口要按库解析，否则 @main 和顶层代码冲突
                .unsafeFlags(["-parse-as-library"])
            ]
        ),
        // 测试直接 @testable import 这个可执行 target —— 省掉一整套 public 标注。
        // Swift 与 Rust 之间的 JSON 契约必须有测试钉住，不然一改就悄悄断。
        .testTarget(
            name: "CloudotTests",
            dependencies: ["Cloudot"],
            path: "Tests/CloudotTests",
            // fixture 是从真实 CLI 抓下来的输出，不是手写的
            resources: [.copy("Fixtures")]
        ),
    ]
)
