import AppKit
import SwiftUI

/// The Settings window: toolbar tabs, one SwiftUI view per tab (mockups
/// 12–13). Managed here rather than as a SwiftUI `Settings` scene, which a
/// menu-bar app cannot reliably open from code.
@MainActor
final class SettingsWindowController: NSWindowController {
    init(general: GeneralSettings, dictionaries: DictionarySettings, data: DataSettings) {
        let tabs = NSTabViewController()
        tabs.tabStyle = .toolbar
        tabs.addTabViewItem(Self.tab("General", symbol: "gearshape", view: general))
        tabs.addTabViewItem(Self.tab("Dictionaries", symbol: "books.vertical", view: dictionaries))
        tabs.addTabViewItem(Self.tab("Data", symbol: "cylinder.split.1x2", view: data))
        let window = NSWindow(contentViewController: tabs)
        window.styleMask = [.titled, .closable]
        window.toolbarStyle = .preference
        window.isReleasedWhenClosed = false
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used")
    }

    func show() {
        guard let window else { return }
        if !window.isVisible { window.center() }
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    private static func tab(_ label: String, symbol: String, view: some View) -> NSTabViewItem {
        let controller = NSHostingController(rootView: view)
        controller.sizingOptions = .preferredContentSize
        let item = NSTabViewItem(viewController: controller)
        item.label = label
        item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
        return item
    }
}

/// General settings (mockup 12): quick search and startup.
struct GeneralSettings: View {
    @Bindable var preferences: Preferences
    /// Sets the shortcut; a message when it cannot be used.
    let setShortcut: (HotKeyBinding) -> String?
    /// Stops the shortcut while a new one is typed, so typing the current
    /// one is recorded instead of opening quick search.
    let pauseShortcut: (Bool) -> Void

    @State private var recording = false
    @State private var monitor: Any?
    @State private var shortcutProblem: String?
    @State private var openAtLogin = LaunchAtLogin.isEnabled
    @State private var loginProblem: String?

    var body: some View {
        Form {
            Section("Quick search") {
                LabeledContent {
                    HStack(spacing: 8) {
                        if recording {
                            Text("Type a shortcut").foregroundStyle(.secondary)
                        } else {
                            HStack(spacing: 4) {
                                ForEach(Array(preferences.hotKey.symbols.enumerated()), id: \.offset) {
                                    KeyCap(label: $0.element, size: .large)
                                }
                            }
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel(preferences.hotKey.display)
                        }
                        Button(recording ? "Cancel" : "Change") { recording ? stopRecording() : startRecording() }
                    }
                } label: {
                    Text("Global shortcut")
                    Text(shortcutProblem ?? "Opens quick search from anywhere on your Mac.")
                        .foregroundStyle(shortcutProblem == nil ? Color.secondary : Palette.warn)
                }
                Toggle("Show 文 in the menu bar", isOn: $preferences.showMenuBarIcon)
                Picker(selection: $preferences.afterSave) {
                    ForEach(AfterSave.allCases) { Text($0.title).tag($0) }
                } label: {
                    Text("After saving a word")
                    Text("What the panel does next.")
                }
            }
            Section("Display") {
                Picker("Pinyin style", selection: $preferences.pinyinStyle) {
                    ForEach(PinyinStyle.allCases) { Text($0.title).tag($0) }
                }
                Toggle("Show traditional characters in lists", isOn: $preferences.showTraditional)
            }
            Section("Startup") {
                Toggle(isOn: $openAtLogin) {
                    Text("Open Shouci at login")
                    if let loginProblem {
                        Text(loginProblem).foregroundStyle(Palette.warn)
                    }
                }
                .onChange(of: openAtLogin) { _, enabled in setOpenAtLogin(enabled) }
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fixedSize(horizontal: false, vertical: true)
        .onDisappear(perform: stopRecording)
    }

    private func startRecording() {
        shortcutProblem = nil
        recording = true
        pauseShortcut(true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            record(event)
            return nil
        }
    }

    private func stopRecording() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        if recording { pauseShortcut(false) }
        recording = false
    }

    private func record(_ event: NSEvent) {
        if event.keyCode == 53, event.modifierFlags.intersection(.deviceIndependentFlagsMask).isEmpty {
            stopRecording()
            return
        }
        guard let binding = HotKeyBinding(event: event), binding.isValid else {
            shortcutProblem = "Use Control or Command with a key (Option alone isn’t allowed)."
            return
        }
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        recording = false
        shortcutProblem = setShortcut(binding)
    }

    private func setOpenAtLogin(_ enabled: Bool) {
        do {
            try LaunchAtLogin.setEnabled(enabled)
            loginProblem = nil
        } catch {
            loginProblem = error.localizedDescription
            openAtLogin = LaunchAtLogin.isEnabled
        }
    }
}
