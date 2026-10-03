import AppKit
import OSLog
import ShouciCore

let log = Logger(subsystem: "com.zacharyzampa.shouci", category: "app")

@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let app = AppModel()
    private let preferences = Preferences()
    private let hotKey = HotKey()
    private var statusItem: StatusItemController?
    private var quickSearch: QuickSearchController?
    private var settings: SettingsWindowController?
    private var library: LibraryWindowController?
    private lazy var libraryModel = LibraryModel(app: app, preferences: preferences)

    static func main() {
        let application = NSApplication.shared
        let delegate = AppDelegate()
        application.delegate = delegate
        application.run()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        if let other = otherInstance() {
            log.info("Shouci is already running; quitting this copy")
            other.activate()
            NSApp.terminate(nil)
            return
        }
        NSApp.mainMenu = mainMenu()

        let search = QuickSearch(app: app, preferences: preferences)
        search.openLibrary = { [weak self] id in self?.openLibrary(showing: id) }
        let quickSearch = QuickSearchController(model: search) { [weak self] in self?.showSettings() }
        let statusItem = StatusItemController(menu: statusMenu()) { [weak quickSearch] in quickSearch?.toggle() }
        quickSearch.anchor = { [weak statusItem] in statusItem?.anchor }
        quickSearch.visibilityChanged = { [weak statusItem] in statusItem?.setHighlighted($0) }
        statusItem.isVisible = preferences.showMenuBarIcon
        preferences.showMenuBarIconChanged = { [weak statusItem] in statusItem?.isVisible = $0 }
        self.quickSearch = quickSearch
        self.statusItem = statusItem

        if let problem = setShortcut(preferences.hotKey) {
            app.post("The shortcut \(preferences.hotKey.display) isn’t working: \(problem) Choose another in Settings.")
        }
        app.start()
    }

    /// Opening Shouci again (from Finder, Spotlight, or the Dock) opens the
    /// library window.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        openLibrary()
        return false
    }

    @objc func openLibrary(_ sender: Any? = nil) {
        openLibrary(showing: nil)
    }

    private func openLibrary(showing id: Int64?) {
        quickSearch?.hide()
        if library == nil { library = LibraryWindowController(model: libraryModel) }
        library?.show()
        if let id {
            Task {
                await libraryModel.reload()
                libraryModel.reveal(id)
            }
        }
    }

    @objc func newWord(_ sender: Any?) {
        openLibrary()
        libraryModel.newWord()
    }

    @objc func importWords(_ sender: Any?) {
        openLibrary()
        libraryModel.transfer = .importing
    }

    @objc func exportWords(_ sender: Any?) {
        openLibrary()
        libraryModel.transfer = .exporting
    }

    @objc func findInLibrary(_ sender: Any?) {
        openLibrary()
        libraryModel.focusSearch()
    }

    /// Registers `binding` and keeps it; a message for people when it fails
    /// (the previous shortcut stays).
    private func setShortcut(_ binding: HotKeyBinding) -> String? {
        do {
            try hotKey.register(binding) { [weak self] in self?.quickSearch?.toggle() }
            preferences.setHotKey(binding)
            statusItem?.setShortcut(binding.display)
            return nil
        } catch {
            log.error("shortcut \(binding.display, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
            if binding != preferences.hotKey {
                try? hotKey.register(preferences.hotKey) { [weak self] in self?.quickSearch?.toggle() }
            }
            return error.localizedDescription
        }
    }

    @objc func showSettings(_ sender: Any? = nil) {
        quickSearch?.hide()
        if settings == nil {
            let general = GeneralSettings(
                preferences: preferences,
                setShortcut: { [weak self] in self?.setShortcut($0) },
                pauseShortcut: { [weak self] paused in
                    guard let self else { return }
                    if paused {
                        hotKey.unregister()
                    } else {
                        _ = setShortcut(preferences.hotKey)
                    }
                })
            settings = SettingsWindowController(
                general: general,
                dictionaries: DictionarySettings(app: app),
                data: DataSettings(
                    app: app,
                    openImport: { [weak self] in self?.importWords(nil) },
                    openExport: { [weak self] in self?.exportWords(nil) }))
        }
        settings?.show()
    }

    private func otherInstance() -> NSRunningApplication? {
        guard let id = Bundle.main.bundleIdentifier else { return nil }
        return NSRunningApplication.runningApplications(withBundleIdentifier: id)
            .first { $0 != .current }
    }

    // MARK: Menus

    private func statusMenu() -> NSMenu {
        let menu = NSMenu()
        menu.addItem(item("Open Shouci", #selector(openLibrary(_:)), key: "o", target: self))
        menu.addItem(item("Settings…", #selector(showSettings(_:)), key: ",", target: self))
        menu.addItem(.separator())
        menu.addItem(item("Quit Shouci", #selector(NSApplication.terminate(_:)), key: "q"))
        return menu
    }

    /// Shown while the library window is open. Otherwise Shouci has no menu
    /// bar of its own, but key equivalents still go through it: without an
    /// Edit menu, ⌘V does nothing in Settings.
    private func mainMenu() -> NSMenu {
        let main = NSMenu()
        let appMenu = NSMenu(title: "Shouci")
        appMenu.addItem(item("About Shouci", #selector(NSApplication.orderFrontStandardAboutPanel(_:))))
        appMenu.addItem(.separator())
        appMenu.addItem(item("Settings…", #selector(showSettings(_:)), key: ",", target: self))
        appMenu.addItem(.separator())
        appMenu.addItem(item("Quit Shouci", #selector(NSApplication.terminate(_:)), key: "q"))
        let file = NSMenu(title: "File")
        file.addItem(item("New Word", #selector(newWord(_:)), key: "n", target: self))
        file.addItem(item("Open Shouci", #selector(openLibrary(_:)), key: "o", target: self))
        file.addItem(.separator())
        file.addItem(item("Import…", #selector(importWords(_:)), key: "I", target: self))
        file.addItem(item("Export…", #selector(exportWords(_:)), key: "E", target: self))
        file.addItem(.separator())
        file.addItem(item("Close", #selector(NSWindow.performClose(_:)), key: "w"))
        let edit = NSMenu(title: "Edit")
        edit.addItem(item("Undo", Selector(("undo:")), key: "z"))
        edit.addItem(item("Redo", Selector(("redo:")), key: "Z"))
        edit.addItem(.separator())
        edit.addItem(item("Cut", #selector(NSText.cut(_:)), key: "x"))
        edit.addItem(item("Copy", #selector(NSText.copy(_:)), key: "c"))
        edit.addItem(item("Paste", #selector(NSText.paste(_:)), key: "v"))
        edit.addItem(item("Select All", #selector(NSText.selectAll(_:)), key: "a"))
        edit.addItem(.separator())
        edit.addItem(item("Find", #selector(findInLibrary(_:)), key: "f", target: self))
        let window = NSMenu(title: "Window")
        window.addItem(item("Minimize", #selector(NSWindow.performMiniaturize(_:)), key: "m"))
        window.addItem(item("Zoom", #selector(NSWindow.performZoom(_:))))
        window.addItem(.separator())
        window.addItem(item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:))))
        NSApp.windowsMenu = window
        for submenu in [appMenu, file, edit, window] {
            let holder = NSMenuItem()
            holder.submenu = submenu
            main.addItem(holder)
        }
        return main
    }

    private func item(_ title: String, _ action: Selector, key: String = "", target: AnyObject? = nil) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.target = target
        return item
    }
}
