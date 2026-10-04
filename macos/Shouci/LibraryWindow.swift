import AppKit
import ShouciCore
import SwiftUI

/// The library window. While it is open Shouci is a regular app, with a
/// Dock icon and menus; closing it goes back to the menu bar only.
@MainActor
final class LibraryWindowController: NSWindowController, NSWindowDelegate {
    let model: LibraryModel

    init(model: LibraryModel) {
        self.model = model
        let hosting = NSHostingController(rootView: LibraryView(model: model))
        hosting.sizingOptions = []
        hosting.sceneBridgingOptions = [.toolbars, .title]
        let window = NSWindow(contentViewController: hosting)
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.toolbarStyle = .unified
        window.title = "Shouci"
        window.minSize = NSSize(width: 900, height: 540)
        window.setContentSize(NSSize(width: 1200, height: 760))
        window.center()
        window.setFrameAutosaveName("ShouciLibrary")
        window.isReleasedWhenClosed = false
        super.init(window: window)
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used")
    }

    func show() {
        guard let window else { return }
        if !window.isVisible {
            model.startWatching()
            model.focusSearch()
            Task { await model.reload() }
        }
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        // A note being typed saves rather than waiting in a hidden window.
        window?.makeFirstResponder(nil)
        model.stopWatching()
        NSApp.setActivationPolicy(.accessory)
    }
}

/// Sidebar, word list, and detail (mockup 01).
struct LibraryView: View {
    @Bindable var model: LibraryModel

    var body: some View {
        NavigationSplitView {
            LibrarySidebar(model: model)
                .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 300)
        } content: {
            WordList(model: model)
                .navigationSplitViewColumnWidth(min: 300, ideal: 380, max: 520)
        } detail: {
            WordDetail(model: model)
                .navigationSplitViewColumnWidth(min: 380, ideal: 560)
        }
        .navigationTitle(model.title)
        .navigationSubtitle(model.subtitle)
        .toolbar { toolbar }
        .sheet(item: $model.editor) { draft in
            WordEditor(model: model, draft: draft)
        }
        .sheet(item: $model.transfer) { transfer in
            TransferSheet(library: model, transfer: transfer)
        }
        .alert(namingTitle, isPresented: isNaming, presenting: model.naming) { _ in
            TextField("Name", text: namingText)
            Button("Cancel", role: .cancel) { model.naming = nil }
            Button("Save") {
                if let naming = model.naming { model.name(naming) }
                model.naming = nil
            }
        }
        .alert("Something went wrong", isPresented: hasProblem) {
            Button("OK") { model.problem = nil }
        } message: {
            Text(model.problem ?? "")
        }
        .task(id: model.app.core != nil) { await model.reload() }
        .onChange(of: model.app.dictionary) {
            if model.isSearching { model.search() }
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            Button {
                model.transfer = .importing
            } label: {
                Label("Import", systemImage: "square.and.arrow.down")
            }
            .help("Import from Pleco or Anki (⇧⌘I)")
            Button {
                model.transfer = .exporting
            } label: {
                Label("Export", systemImage: "square.and.arrow.up")
            }
            .help("Export to Pleco or Anki (⇧⌘E)")
            // One item whatever the chip does: adding a toolbar item would
            // rebuild the search field while it is being typed in.
            HStack(spacing: 8) {
                if model.isSearching, let kind = model.kind ?? model.searchedKind {
                    KindMenu(kind: $model.kind, shown: kind)
                }
                SearchField(
                    text: $model.query,
                    look: .toolbar,
                    focusRequests: model.focusSearchRequests,
                    onMove: model.move(by:),
                    onSubmit: model.submit,
                    onEscape: { model.query = "" }
                )
                .frame(width: 240)
            }
            Button {
                model.newWord()
            } label: {
                Label("Add Word", systemImage: "plus")
            }
            .labelStyle(.titleAndIcon)
            .help("Add a word by hand (⌘N)")
        }
    }

    private var hasProblem: Binding<Bool> {
        Binding(get: { model.problem != nil }, set: { if !$0 { model.problem = nil } })
    }

    private var isNaming: Binding<Bool> {
        Binding(get: { model.naming != nil }, set: { if !$0 { model.naming = nil } })
    }

    private var namingText: Binding<String> {
        Binding(get: { model.naming?.text ?? "" }, set: { model.naming?.text = $0 })
    }

    private var namingTitle: String {
        switch model.naming?.purpose {
        case .renameCollection: "Rename Collection"
        case .renameTag: "Rename Tag"
        default: "New Collection"
        }
    }
}
