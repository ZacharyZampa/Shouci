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
        // `delegate` is weak, and optimized builds may release a local after
        // its last use: keep it alive for as long as the app runs.
        withExtendedLifetime(delegate) { application.run() }
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
        LaunchAtLogin.refresh()
    }

    /// Quitting saves a note still being typed in the library window, and
    /// waits for changes on their way to the library.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard library != nil else { return .terminateNow }
        Task {
            await libraryModel.finishChanges()
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
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

    @objc func fileInCollection(_ sender: Any?) {
        openLibrary()
        libraryModel.startFiling()
    }

    // MARK: Word menu

    /// The words a Word menu command acts on: the library window's
    /// selection, while that window has the keyboard, no sheet is open, and
    /// no field is being typed in (there ⌘⌫ and ⌘E edit the text instead).
    private var menuWords: [ItemView] {
        guard let window = library?.window, window.isKeyWindow, !libraryModel.showsSheet,
            !(window.firstResponder is NSTextView)
        else { return [] }
        return libraryModel.selectedItems
    }

    @objc func editWord(_ sender: Any?) {
        let words = menuWords
        if words.count == 1, let word = words.first { libraryModel.edit(word) }
    }

    // Each turns back once every word has it (`reviewToggle` and the others).

    @objc func markWords(_ sender: Any?) {
        let words = menuWords
        libraryModel.perform(words.reviewToggle, on: words.map(\.id))
    }

    @objc func archiveWords(_ sender: Any?) {
        let words = menuWords
        libraryModel.perform(words.archiveToggle, on: words.map(\.id))
    }

    @objc func trashWords(_ sender: Any?) {
        let words = menuWords
        libraryModel.perform(words.trashToggle, on: words.map(\.id))
    }

    @objc func findInLibrary(_ sender: Any?) {
        openLibrary()
        libraryModel.focusSearch()
    }

    /// The standard About panel, with who made Shouci and how to support it.
    @objc func showAbout(_ sender: Any?) {
        NSApp.activate()
        NSApp.orderFrontStandardAboutPanel(options: [.credits: Self.credits()])
    }

    private static func credits() -> NSAttributedString {
        let centered = NSMutableParagraphStyle()
        centered.alignment = .center
        let plain: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
            .foregroundColor: NSColor.secondaryLabelColor,
            .paragraphStyle: centered,
        ]
        func link(_ title: String, _ url: String) -> NSAttributedString {
            var attributes = plain
            attributes[.link] = URL(string: url)
            return NSAttributedString(string: title, attributes: attributes)
        }
        let text = NSMutableAttributedString(string: "Made by Zachary Zampa\n", attributes: plain)
        text.append(link("Website", "https://zacharyzampa.github.io/ZampaPortfolio"))
        text.append(NSAttributedString(string: "  ·  ", attributes: plain))
        text.append(link("Buy me a coffee", "https://buymeacoffee.com/zacharyzampa"))
        return text
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
        appMenu.addItem(item("About Shouci", #selector(showAbout(_:)), target: self))
        appMenu.addItem(.separator())
        appMenu.addItem(item("Settings…", #selector(showSettings(_:)), key: ",", target: self))
        appMenu.addItem(.separator())
        appMenu.addItem(item("Hide Shouci", #selector(NSApplication.hide(_:)), key: "h"))
        appMenu.addItem(
            item("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), key: "h", modifiers: [.command, .option]))
        appMenu.addItem(item("Show All", #selector(NSApplication.unhideAllApplications(_:))))
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
        // AppKit names these Show or Hide Sidebar, and Enter or Exit Full
        // Screen, as they apply.
        let view = NSMenu(title: "View")
        view.addItem(
            item("Show Sidebar", #selector(NSSplitViewController.toggleSidebar(_:)), key: "s", modifiers: [.command, .control]))
        view.addItem(.separator())
        view.addItem(
            item("Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), key: "f", modifiers: [.command, .control]))
        // The selected words. Titles follow the selection (`validateMenuItem`).
        let word = NSMenu(title: "Word")
        word.addItem(item("Edit Word…", #selector(editWord(_:)), key: "e", target: self))
        word.addItem(item("File in Collection…", #selector(fileInCollection(_:)), key: "C", target: self))
        word.addItem(.separator())
        word.addItem(item(BulkAction.setVerification(.needsReview).title, #selector(markWords(_:)), key: "R", target: self))
        word.addItem(item(BulkAction.archive.title, #selector(archiveWords(_:)), key: "a", modifiers: [.command, .control], target: self))
        word.addItem(.separator())
        word.addItem(item(BulkAction.trash.title, #selector(trashWords(_:)), key: "\u{8}", target: self))
        let window = NSMenu(title: "Window")
        window.addItem(item("Minimize", #selector(NSWindow.performMiniaturize(_:)), key: "m"))
        window.addItem(item("Zoom", #selector(NSWindow.performZoom(_:))))
        window.addItem(.separator())
        window.addItem(item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:))))
        NSApp.windowsMenu = window
        // Empty but for the search field macOS puts in a Help menu, which
        // finds any command by name.
        let help = NSMenu(title: "Help")
        NSApp.helpMenu = help
        for submenu in [appMenu, file, edit, view, word, window, help] {
            let holder = NSMenuItem()
            holder.submenu = submenu
            main.addItem(holder)
        }
        return main
    }

    private func item(
        _ title: String, _ action: Selector, key: String = "", modifiers: NSEvent.ModifierFlags = .command,
        target: AnyObject? = nil
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = modifiers
        item.target = target
        return item
    }
}

extension AppDelegate: NSMenuItemValidation {
    /// The Word menu acts on the selection, and says what it will do to it.
    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        let words = menuWords
        let trashed = words.allTrashed
        switch item.action {
        case #selector(editWord(_:)):
            return words.count == 1 && !trashed
        case #selector(markWords(_:)):
            item.title = words.reviewToggle.title
            return !words.isEmpty && !trashed
        case #selector(archiveWords(_:)):
            item.title = words.archiveToggle.title
            return !words.isEmpty && !trashed
        case #selector(trashWords(_:)):
            item.title = words.trashToggle.title
            return !words.isEmpty
        default:
            return true
        }
    }
}
