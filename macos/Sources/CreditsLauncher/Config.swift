import Foundation

/// The config file: `url` names the server and `theme` the palette.
/// Found at `$CREDITS_CONFIG`, else `$XDG_CONFIG_HOME/credits/config.toml`,
/// else `~/.config/credits/config.toml`.
enum CreditsConfig {
    static var path: URL {
        let env = ProcessInfo.processInfo.environment.filter { !$0.value.isEmpty }
        return env["CREDITS_CONFIG"].map { URL(fileURLWithPath: $0) }
            ?? env["XDG_CONFIG_HOME"].map { URL(fileURLWithPath: $0).appendingPathComponent("credits/config.toml") }
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/credits/config.toml")
    }

    /// A top-level string value, read line by line (the file is two flat keys).
    static func value(_ key: String) -> String? {
        guard let text = try? String(contentsOf: path, encoding: .utf8) else { return nil }
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            if let value = parse(line, key) { return value }
        }
        return nil
    }

    private static func parse(_ line: Substring, _ key: String) -> String? {
        let parts = line.split(separator: "=", maxSplits: 1)
        guard parts.count == 2, parts[0].trimmingCharacters(in: .whitespaces) == key else { return nil }
        return parts[1].split(separator: "#").first?
            .trimmingCharacters(in: .whitespaces)
            .trimmingCharacters(in: CharacterSet(charactersIn: "\""))
    }
}
