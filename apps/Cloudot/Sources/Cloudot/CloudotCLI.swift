import Foundation

/// 出错时抛出的类型。`kind` 让界面能按错误种类给不同的处置引导。
enum CloudotError: LocalizedError {
    /// 找不到 cloudot 可执行文件
    case binaryNotFound([String])
    /// CLI 正常返回了错误信封
    case reported(ErrorResult)
    /// CLI 没有输出合法 JSON（崩了、或者版本对不上）
    case badOutput(command: String, status: Int32, detail: String)

    var errorDescription: String? {
        switch self {
        case .binaryNotFound(let searched):
            "找不到 cloudot 可执行文件。找过：\n" + searched.joined(separator: "\n")
        case .reported(let result):
            result.message
        case .badOutput(let command, let status, let detail):
            "cloudot \(command) 退出码 \(status)，输出不是预期的 JSON：\n\(detail)"
        }
    }

    /// 界面标题用的短摘要。
    var summary: String {
        switch self {
        case .binaryNotFound: "找不到 cloudot"
        case .reported(let result): result.summary
        case .badOutput: "命令输出异常"
        }
    }

    var kind: ErrorKind {
        switch self {
        case .reported(let result): result.kind
        default: .other
        }
    }
}

/// 对 `cloudot` CLI 的调用层。
///
/// GUI 不复用 Rust 的 core，而是起进程跑 CLI：进程隔离，界面崩了动不到用户的配置，
/// 而且 CLI 的 `--json` 本来就是为三端共用设计的契约。
struct CloudotCLI: Sendable {
    let executable: URL

    /// 找二进制：优先用 .app 里自带的那份，这样用户不必先装 CLI。
    static func locate() -> Result<CloudotCLI, CloudotError> {
        var candidates: [URL] = []
        if let bundled = Bundle.main.url(forResource: "cloudot", withExtension: nil) {
            candidates.append(bundled)
        }
        let home = FileManager.default.homeDirectoryForCurrentUser
        candidates += [
            home.appending(path: ".cargo/bin/cloudot"),
            URL(filePath: "/opt/homebrew/bin/cloudot"),
            URL(filePath: "/usr/local/bin/cloudot"),
        ]
        // 允许用 defaults write 指定，方便开发时指向 target/debug
        if let override = UserDefaults.standard.string(forKey: "cloudotBinaryPath") {
            candidates.insert(URL(filePath: override), at: 0)
        }

        for url in candidates where FileManager.default.isExecutableFile(atPath: url.path) {
            return .success(CloudotCLI(executable: url))
        }
        return .failure(.binaryNotFound(candidates.map(\.path)))
    }

    // MARK: - 查询

    func status() async throws -> Status {
        try await call(["status"], as: Status.self)
    }

    func doctor(net: Bool) async throws -> DoctorReport {
        try await call(net ? ["doctor", "--net"] : ["doctor"], as: DoctorReport.self)
    }

    func apps() async throws -> [AppListing] {
        try await call(["apps"], as: [AppListing].self)
    }

    func backups() async throws -> BackupSet {
        try await call(["backups"], as: BackupSet.self)
    }

    // MARK: - 改动

    func sync() async throws -> SyncResult {
        try await call(["sync"], as: SyncResult.self)
    }

    /// 刻意不暴露 `--force`：那个开关会用 store 覆盖本地实体文件，只留在 CLI 里。
    func apply() async throws -> ApplyResult {
        try await call(["apply"], as: ApplyResult.self)
    }

    /// 同样不暴露 `--allow-secrets`。
    func add(_ appID: String) async throws -> [AddResult] {
        try await call(["add", appID], as: [AddResult].self)
    }

    func unadopt(_ appID: String) async throws -> UnadoptResult {
        try await call(["unadopt", appID], as: UnadoptResult.self)
    }

    func pruneBackups(keep: Int) async throws -> PruneResult {
        try await call(["backups", "prune", "--keep", String(keep)], as: PruneResult.self)
    }

    // MARK: - 内部

    private func call<R: Decodable>(_ args: [String], as type: R.Type) async throws -> R {
        let output = try await Self.execute(executable, args + ["--json"])
        let decoder = JSONDecoder()

        // 先尝试按成功信封解。注意不能先看退出码：`doctor` 在有 error 级检查项时
        // 会以非零码退出，但输出仍然是合法的成功信封。
        if let envelope = try? decoder.decode(Envelope<R>.self, from: output.stdout),
           envelope.ok {
            return envelope.result
        }
        if let envelope = try? decoder.decode(Envelope<ErrorResult>.self, from: output.stdout),
           !envelope.ok {
            throw CloudotError.reported(envelope.result)
        }

        let detail = [String(data: output.stdout, encoding: .utf8), output.stderr]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
        throw CloudotError.badOutput(
            command: args.joined(separator: " "),
            status: output.status,
            detail: detail.isEmpty ? "（没有输出）" : detail
        )
    }

    private struct Output: Sendable {
        let stdout: Data
        let stderr: String
        let status: Int32
    }

    private static func execute(_ executable: URL, _ args: [String]) async throws -> Output {
        try await Task.detached(priority: .userInitiated) {
            let process = Process()
            process.executableURL = executable
            process.arguments = args
            process.environment = augmentedEnvironment()

            let out = Pipe()
            let err = Pipe()
            process.standardOutput = out
            process.standardError = err

            try process.run()
            // --json 模式下错误也走 stdout，stderr 只在崩溃时有内容且很小，
            // 所以先读干 stdout 不会死锁。
            let stdout = out.fileHandleForReading.readDataToEndOfFile()
            let stderrData = err.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()

            return Output(
                stdout: stdout,
                stderr: String(data: stderrData, encoding: .utf8) ?? "",
                status: process.terminationStatus
            )
        }.value
    }

    /// GUI 进程从 Finder 启动时 PATH 很窄，而 cloudot 要调 `git`。
    private static func augmentedEnvironment() -> [String: String] {
        var env = ProcessInfo.processInfo.environment
        let existing = env["PATH"]?.split(separator: ":").map(String.init) ?? []
        let extras = ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/opt/homebrew/bin", "/usr/local/bin"]
        var seen = Set<String>()
        env["PATH"] = (existing + extras).filter { seen.insert($0).inserted }
            .joined(separator: ":")
        return env
    }
}
