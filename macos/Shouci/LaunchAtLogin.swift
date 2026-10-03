import Foundation

/// Open at login through a user LaunchAgent. Unlike `SMAppService`, it
/// works for an app that is only signed to run locally. The label is the
/// one the menu-bar app has always used, so the setting carries over.
enum LaunchAtLogin {
    static let label = "com.zacharyzampa.shouci"

    static var plist: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appending(path: "Library/LaunchAgents/\(label).plist")
    }

    static var isEnabled: Bool {
        FileManager.default.fileExists(atPath: plist.path)
    }

    /// Takes effect at the next login. Opening through `open` lets Launch
    /// Services start the app, so a copy that is already running is reused.
    static func setEnabled(_ enabled: Bool) throws {
        guard enabled else {
            if isEnabled { try FileManager.default.removeItem(at: plist) }
            return
        }
        let agent: [String: Any] = [
            "Label": label,
            "ProgramArguments": ["/usr/bin/open", Bundle.main.bundlePath],
            "RunAtLoad": true,
        ]
        let data = try PropertyListSerialization.data(fromPropertyList: agent, format: .xml, options: 0)
        try FileManager.default.createDirectory(
            at: plist.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: plist, options: .atomic)
    }
}
