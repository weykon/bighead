import Foundation

@MainActor
protocol RuntimeProcessDelegate: AnyObject {
    func runtimeProcess(_ process: RuntimeProcessController, didLog line: String)
    func runtimeProcess(_ process: RuntimeProcessController, didChangeRunning running: Bool)
}

@MainActor
final class RuntimeProcessController {
    weak var delegate: RuntimeProcessDelegate?
    private var process: Process?
    private var stdout: Pipe?
    private var stderr: Pipe?

    var isRunning: Bool {
        process?.isRunning == true
    }

    func start() {
        guard !isRunning else { return }
        let launch = resolveLaunch()
        let proc = Process()
        proc.executableURL = launch.executable
        proc.arguments = launch.arguments
        proc.currentDirectoryURL = launch.workingDirectory
        proc.environment = ProcessInfo.processInfo.environment.merging([
            "BIG_HEAD_DESKTOP": "1"
        ]) { current, _ in current }

        let out = Pipe()
        let err = Pipe()
        proc.standardOutput = out
        proc.standardError = err
        stdout = out
        stderr = err
        attach(pipe: out, prefix: "runtime")
        attach(pipe: err, prefix: "runtime")

        proc.terminationHandler = { [weak self] process in
            Task { @MainActor in
                guard let self else { return }
                self.delegate?.runtimeProcess(self, didLog: "runtime exited with status \(process.terminationStatus)")
                self.process = nil
                self.delegate?.runtimeProcess(self, didChangeRunning: false)
            }
        }

        do {
            try proc.run()
            process = proc
            delegate?.runtimeProcess(self, didChangeRunning: true)
            delegate?.runtimeProcess(self, didLog: "started bighead via \(launch.label)")
            delegate?.runtimeProcess(self, didLog: "runtime data directory: \(launch.workingDirectory.path)")
        } catch {
            delegate?.runtimeProcess(self, didLog: "failed to start runtime: \(error.localizedDescription)")
        }
    }

    func stop() {
        guard let process else { return }
        process.terminate()
        self.process = nil
        delegate?.runtimeProcess(self, didChangeRunning: false)
    }

    private func attach(pipe: Pipe, prefix: String) {
        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty,
                  let text = String(data: data, encoding: .utf8)
            else {
                return
            }
            Task { @MainActor in
                guard let self else { return }
                for line in text.split(whereSeparator: \.isNewline) {
                    self.delegate?.runtimeProcess(self, didLog: "\(prefix): \(line)")
                }
            }
        }
    }

    private struct RuntimeLaunch {
        var executable: URL
        var arguments: [String]
        var workingDirectory: URL
        var label: String
    }

    private func resolveLaunch() -> RuntimeLaunch {
        let support = appSupportDirectory()

        if let runtime = bundledRuntime() {
            return RuntimeLaunch(
                executable: runtime,
                arguments: ["--view-server", "--headless"],
                workingDirectory: support,
                label: "bundled runtime"
            )
        }

        if let env = ProcessInfo.processInfo.environment["BIG_HEAD_RUNTIME"], !env.isEmpty {
            return RuntimeLaunch(
                executable: URL(fileURLWithPath: env),
                arguments: ["--view-server", "--headless"],
                workingDirectory: support,
                label: "BIG_HEAD_RUNTIME"
            )
        }

        let root = resolveRepoRoot()
        return RuntimeLaunch(
            executable: URL(fileURLWithPath: "/usr/bin/env"),
            arguments: ["cargo", "run", "--", "--view-server", "--headless"],
            workingDirectory: root,
            label: "cargo at \(root.path)"
        )
    }

    private func bundledRuntime() -> URL? {
        let candidates = [
            Bundle.main.resourceURL?.appendingPathComponent("big-head-runtime"),
            Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/big-head-runtime")
        ].compactMap { $0 }

        for candidate in candidates {
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }

    private func appSupportDirectory() -> URL {
        EnvironmentSettingsStore.supportDirectory()
    }

    private func resolveRepoRoot() -> URL {
        if let env = ProcessInfo.processInfo.environment["BIG_HEAD_ROOT"], !env.isEmpty {
            return URL(fileURLWithPath: env)
        }

        let cwd = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        let candidates = [
            cwd,
            cwd.deletingLastPathComponent().deletingLastPathComponent(),
            URL(fileURLWithPath: Bundle.main.bundlePath)
                .deletingLastPathComponent()
                .deletingLastPathComponent()
                .deletingLastPathComponent()
        ]

        for candidate in candidates {
            if FileManager.default.fileExists(atPath: candidate.appendingPathComponent("Cargo.toml").path) {
                return candidate
            }
        }
        return cwd
    }
}
