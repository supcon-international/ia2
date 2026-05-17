// BackendSupervisor — owns the lifecycle of the Rust HTTP server.
//
// Responsibilities:
//   1. Spawn the bundled `ia2-server` binary with the
//      desktop-mode flags (`--bind 127.0.0.1:0 --print-url
//      --static-dir <bundled-dist>`).
//   2. Block until the server prints one line to stdout — that line
//      IS the base URL the WebView will load. Anything else (tracing
//      logs, panic backtraces) goes to stderr and is forwarded to
//      Console for debugging.
//   3. On window close / app terminate, send SIGTERM and reap. If the
//      child doesn't exit within `shutdownGraceSeconds`, SIGKILL.
//
// The contract with the Rust side is intentionally tiny: stdout is a
// machine channel (URL goes here, single line, then nothing); stderr
// is a human channel (everything else). See
// `crates/server/src/main.rs` for the matching producer side.

import Foundation

final class BackendSupervisor {
    /// Resolved at first `start()` and cached.
    private var process: Process?
    private var stdoutPipe: Pipe?
    private var stderrPipe: Pipe?
    /// URL printed by the server on stdout. `nil` until handshake.
    private(set) var baseURL: URL?

    /// How long to wait for the server's stdout handshake (URL line)
    /// before giving up. Real-world cold start of the Rust binary +
    /// bind to localhost is comfortably under 200 ms; 5 s is generous.
    private let startupTimeout: TimeInterval = 5.0

    /// How long to wait for graceful SIGTERM shutdown before
    /// escalating to SIGKILL.
    private let shutdownGraceSeconds: TimeInterval = 2.0

    /// Boot the server and return the URL it announced on stdout.
    /// Throws if the binary is missing, can't bind, or doesn't print a
    /// URL within `startupTimeout`. Synchronous on the calling thread
    /// — designed to run on the main thread before the window opens,
    /// so the UI never races the backend.
    func start() throws -> URL {
        if let url = baseURL { return url }

        guard let binaryURL = Self.locateServerBinary() else {
            throw SupervisorError.binaryNotFound
        }
        let staticDir = Self.locateStaticDir()

        let process = Process()
        process.executableURL = binaryURL
        // `--parent-pid` lets the server self-reap if the shell is
        // SIGKILLed or otherwise dies without running cleanup —
        // see crates/server/src/main.rs::parent_watchdog. Belt-and-
        // suspenders with the signal handler in main.swift.
        let ourPid = ProcessInfo.processInfo.processIdentifier
        var args = [
            "--bind", "127.0.0.1:0",
            "--print-url",
            "--parent-pid", String(ourPid),
        ]
        if let dir = staticDir {
            args.append(contentsOf: ["--static-dir", dir.path])
        }
        process.arguments = args

        // Inherit a clean env. We deliberately don't pass through the
        // user's shell PATH — the bundle ships the binary alongside
        // and resolves it absolutely.
        process.environment = [
            "RUST_LOG": ProcessInfo.processInfo.environment["RUST_LOG"]
                ?? "server=info,tower_http=info,info",
        ]

        let outPipe = Pipe()
        let errPipe = Pipe()
        let stdinPipe = Pipe()
        process.standardOutput = outPipe
        process.standardError = errPipe
        // Connect a stdin pipe we never write to. The child's
        // parent-watchdog blocks on `read(0, ...)` and exits the
        // instant that read returns 0 (EOF). When this shell dies
        // for ANY reason — graceful Cmd-Q, SIGTERM, SIGKILL, panic,
        // OOM kill, force-quit — the OS closes the write end of
        // this pipe, the child's read unblocks with 0, and the
        // child exits. Bulletproof regardless of launch method
        // (direct, `open`, launchctl, ssh).
        //
        // The Swift side retains `stdinPipe` indirectly via the
        // Process; we don't keep a separate reference because
        // anything we do to the FileHandleForWriting (close,
        // release) would trip the watchdog prematurely.
        process.standardInput = stdinPipe

        // Forward the child's stderr to our own stderr live, so
        // Console.app shows the server's logs alongside the shell's.
        errPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            if !data.isEmpty {
                FileHandle.standardError.write(data)
            }
        }

        // Detect unexpected exit (e.g. port collision, panic).
        process.terminationHandler = { proc in
            NSLog(
                "ia2-server exited: status=\(proc.terminationStatus) reason=\(proc.terminationReason.rawValue)"
            )
        }

        try process.run()
        self.process = process
        self.stdoutPipe = outPipe
        self.stderrPipe = errPipe

        // Read stdout synchronously until we get a complete line or
        // we time out. The server prints exactly one line — the URL
        // — and then writes nothing else to stdout for the rest of
        // its life.
        let deadline = Date().addingTimeInterval(startupTimeout)
        var buffer = Data()
        while Date() < deadline {
            let chunk = outPipe.fileHandleForReading.availableData
            if chunk.isEmpty {
                if !process.isRunning {
                    throw SupervisorError.exitedBeforeHandshake(
                        status: process.terminationStatus
                    )
                }
                // No data yet — spin briefly. Avoids tight loop.
                Thread.sleep(forTimeInterval: 0.02)
                continue
            }
            buffer.append(chunk)
            if let newlineIdx = buffer.firstIndex(of: 0x0A) {
                let lineData = buffer.prefix(upTo: newlineIdx)
                guard
                    let line = String(data: lineData, encoding: .utf8)?
                        .trimmingCharacters(in: .whitespacesAndNewlines),
                    let url = URL(string: line)
                else {
                    throw SupervisorError.malformedURLLine(
                        String(data: buffer, encoding: .utf8) ?? "<non-utf8>"
                    )
                }
                self.baseURL = url
                return url
            }
        }
        throw SupervisorError.startupTimeout
    }

    /// Send SIGTERM, wait, escalate to SIGKILL if needed. Idempotent:
    /// safe to call from any thread, multiple times. Called from
    /// `applicationWillTerminate` and `windowShouldClose`.
    func stop() {
        guard let process = process, process.isRunning else { return }
        process.terminate()  // SIGTERM
        let deadline = Date().addingTimeInterval(shutdownGraceSeconds)
        while Date() < deadline && process.isRunning {
            Thread.sleep(forTimeInterval: 0.05)
        }
        if process.isRunning {
            NSLog("ia2-server didn't exit on SIGTERM, sending SIGKILL")
            kill(process.processIdentifier, SIGKILL)
        }
    }

    // MARK: - Path resolution

    /// Look up the server binary. In a deployed `.app` bundle it
    /// lives in `Contents/MacOS/ia2-server`. During
    /// `swift run` development we walk up to find the Rust target
    /// directory.
    private static func locateServerBinary() -> URL? {
        let bundle = Bundle.main
        // (1) Same directory as the shell binary inside Contents/MacOS.
        if let exec = bundle.executableURL {
            let sibling = exec.deletingLastPathComponent()
                .appendingPathComponent("ia2-server")
            if FileManager.default.isExecutableFile(atPath: sibling.path) {
                return sibling
            }
        }
        // (2) Dev fallback: look in the repo's `target/debug/server`.
        //     Walk up from the executable's directory.
        var dir = bundle.executableURL?.deletingLastPathComponent()
        for _ in 0..<8 {
            guard let d = dir else { break }
            let candidate = d.appendingPathComponent("target/debug/server")
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
            dir = d.deletingLastPathComponent()
        }
        return nil
    }

    /// Look up the bundled static dist. In a deployed bundle it's
    /// `Contents/Resources/web`. During development we walk up to
    /// find `apps/web/dist`.
    private static func locateStaticDir() -> URL? {
        if let resources = Bundle.main.resourceURL {
            let web = resources.appendingPathComponent("web")
            if let isDir = try? web.resourceValues(forKeys: [.isDirectoryKey])
                .isDirectory, isDir == true
            {
                return web
            }
        }
        var dir = Bundle.main.executableURL?.deletingLastPathComponent()
        for _ in 0..<8 {
            guard let d = dir else { break }
            let candidate = d.appendingPathComponent("apps/web/dist")
            if FileManager.default.fileExists(atPath: candidate.path) {
                return candidate
            }
            dir = d.deletingLastPathComponent()
        }
        return nil
    }
}

enum SupervisorError: Error, CustomStringConvertible {
    case binaryNotFound
    case startupTimeout
    case exitedBeforeHandshake(status: Int32)
    case malformedURLLine(String)

    var description: String {
        switch self {
        case .binaryNotFound:
            return
                "ia2-server binary not found (looked in bundle MacOS dir and ../target/debug/server)"
        case .startupTimeout:
            return "ia2-server didn't print a URL within timeout"
        case .exitedBeforeHandshake(let status):
            return "ia2-server exited (status=\(status)) before printing a URL"
        case .malformedURLLine(let line):
            return "ia2-server printed a malformed URL line: \(line)"
        }
    }
}
