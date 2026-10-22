import Foundation

struct BigHeadEnvironment {
    let supportDirectory: URL
    let envFile: URL
    var minimaxAPIKey: String

    var hasAPIKey: Bool {
        !minimaxAPIKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

enum EnvironmentSettingsStore {
    static func supportDirectory() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first ?? URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        let dir = base.appendingPathComponent("BigHead", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    static func load() -> BigHeadEnvironment {
        let dir = supportDirectory()
        let env = dir.appendingPathComponent(".env")
        ensureExample(in: dir)
        let values = parseEnv(file: env)
        return BigHeadEnvironment(
            supportDirectory: dir,
            envFile: env,
            minimaxAPIKey: values["MiniMax_API_KEY"] ?? ""
        )
    }

    static func save(_ environment: BigHeadEnvironment) throws {
        var values = parseEnv(file: environment.envFile)
        values["MiniMax_API_KEY"] = environment.minimaxAPIKey.trimmingCharacters(in: .whitespacesAndNewlines)
        let orderedKeys = ["MiniMax_API_KEY"] + values.keys.filter { $0 != "MiniMax_API_KEY" }.sorted()
        let text = orderedKeys
            .compactMap { key -> String? in
                guard let value = values[key] else { return nil }
                return "\(key)=\(value)"
            }
            .joined(separator: "\n") + "\n"
        try text.write(to: environment.envFile, atomically: true, encoding: .utf8)
        ensureExample(in: environment.supportDirectory)
    }

    private static func parseEnv(file: URL) -> [String: String] {
        guard let text = try? String(contentsOf: file, encoding: .utf8) else {
            return [:]
        }
        var values: [String: String] = [:]
        for rawLine in text.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            if line.isEmpty || line.hasPrefix("#") {
                continue
            }
            guard let eq = line.firstIndex(of: "=") else {
                continue
            }
            let key = String(line[..<eq]).trimmingCharacters(in: .whitespaces)
            var value = String(line[line.index(after: eq)...]).trimmingCharacters(in: .whitespaces)
            if value.count >= 2,
               value.first == "\"",
               value.last == "\"" {
                value.removeFirst()
                value.removeLast()
            }
            values[key] = value
        }
        return values
    }

    private static func ensureExample(in dir: URL) {
        let envPath = dir.appendingPathComponent(".env.example")
        guard !FileManager.default.fileExists(atPath: envPath.path) else { return }
        let text = """
        # Copy this file to .env and fill your provider key.
        MiniMax_API_KEY=
        """
        try? text.write(to: envPath, atomically: true, encoding: .utf8)
    }
}

