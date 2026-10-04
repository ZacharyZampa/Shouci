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
    /// It is found by bundle identifier, not path, so moving or reinstalling
    /// the app keeps the setting working.
    static func setEnabled(_ enabled: Bool) throws {
        guard enabled else {
            if isEnabled { try FileManager.default.removeItem(at: plist) }
            return
        }
        let program = Bundle.main.bundleIdentifier.map { ["/usr/bin/open", "-b", $0] }
            ?? ["/usr/bin/open", Bundle.main.bundlePath]
        let agent: [String: Any] = [
            "Label": label,
            "ProgramArguments": program,
            "RunAtLoad": true,
        ]
        let data = try PropertyListSerialization.data(fromPropertyList: agent, format: .xml, options: 0)
        try FileManager.default.createDirectory(
            at: plist.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: plist, options: .atomic)
    }

    /// Rewrites an agent saved by an older version, which named this app by
    /// path. Any other agent is left alone (a development build must not
    /// take over the installed app's).
    static func refresh() {
        guard let data = try? Data(contentsOf: plist),
            let agent = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
            agent["ProgramArguments"] as? [String] == ["/usr/bin/open", Bundle.main.bundlePath]
        else { return }
        do {
            try setEnabled(true)
        } catch {
            log.error("could not update the login item: \(error.localizedDescription, privacy: .public)")
        }
    }
}
