import AppKit
import Foundation
import Observation
import ShouciCore
import UniformTypeIdentifiers

/// Import (mockup 10): choose a file, see what each line would do, import.
@MainActor
@Observable
final class ImportModel {
    let library: LibraryModel

    private(set) var file: URL?
    /// The format reading the file. Choosing another previews again.
    var connector: String? {
        get { readAs }
        set {
            guard newValue != readAs else { return }
            readAs = newValue
            refresh()
        }
    }
    /// True when no other format reads the file as well.
    private(set) var detected = false
    var policy: ImportPolicy = .merge {
        didSet { if policy != oldValue { refresh() } }
    }
    /// Import the readable lines of a file with error lines.
    var skipErrors: Bool {
        get { forced }
        set {
            guard newValue != forced else { return }
            forced = newValue
            refresh()
        }
    }
    private(set) var preview: ImportPreview?
    private(set) var plan: ImportPlanView?
    private(set) var problem: String?
    private(set) var isWorking = false
    private(set) var summary: TransferSummary?

    // What `connector` and `skipErrors` show. Detecting the format sets
    // these directly, so it doesn't start a second preview.
    private var readAs: String?
    private var forced = false

    @ObservationIgnored private var previewing: Task<Void, Never>?
    /// Counts previews, so only the newest one clears `isWorking`.
    @ObservationIgnored private var generation = 0

    init(library: LibraryModel) {
        self.library = library
    }

    var formats: [ConnectorView] { library.connectors.filter(\.canImport) }

    var fileSize: String? {
        guard let file, let size = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize else { return nil }
        return ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file)
    }

    // MARK: Counts, as the mockup shows them

    var newWords: Int { Int((plan?.counts.inserts ?? 0) - (plan?.counts.unresolved ?? 0)) }
    var needsReview: Int { Int(plan?.counts.unresolved ?? 0) }
    var alreadySaved: Int {
        plan?.lines.filter { line in
            switch line.action {
            case .update: true
            case .skip(_, _, _, let reason, _): reason != .repeatedInFile
            default: false
            }
        }.count ?? 0
    }
    var updates: Int { Int(plan?.counts.updates ?? 0) }
    /// Words that change: added or updated.
    var changing: Int { Int((plan?.counts.inserts ?? 0) + (plan?.counts.updates ?? 0)) }
    var errorLines: [IssueView] { plan?.issues.filter { $0.severity == .error } ?? [] }
    var canImport: Bool {
        plan != nil && plan?.refused == false && changing > 0 && !isWorking && summary == nil
    }

    // MARK: Steps

    func chooseFile() {
        let panel = NSOpenPanel()
        panel.message = "Choose a file exported from Pleco or Anki."
        panel.prompt = "Choose"
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        let extensions = Set(formats.flatMap(\.extensions))
        panel.allowedContentTypes = extensions.compactMap { UTType(filenameExtension: $0) }
        panel.allowsOtherFileTypes = true
        if panel.runModal() == .OK, let url = panel.url { open(url) }
    }

    func open(_ url: URL) {
        file = url
        plan = nil
        preview = nil
        summary = nil
        problem = nil
        forced = false
        detect()
    }

    /// Reads the file as whichever format fits it best (the core tries each).
    private func detect() {
        guard let file else { return }
        let path = file.path
        let policy = self.policy
        previewing?.cancel()
        isWorking = true
        generation += 1
        let mine = generation
        previewing = Task {
            defer { if generation == mine { isWorking = false } }
            do {
                let found = try await library.app.call {
                    try $0.detectImport(path: path, policy: policy, force: false)
                }
                guard !Task.isCancelled else { return }
                let plan = found.preview.view()
                detected = found.unambiguous
                readAs = plan.connectorId
                preview = found.preview
                self.plan = plan
            } catch {
                guard !Task.isCancelled else { return }
                problem = describe(error)
            }
        }
    }

    private func refresh() {
        guard let file, let connector else { return }
        let path = file.path
        let policy = self.policy
        let force = skipErrors
        previewing?.cancel()
        isWorking = true
        problem = nil
        generation += 1
        let mine = generation
        previewing = Task {
            defer { if generation == mine { isWorking = false } }
            do {
                let preview = try await library.app.call {
                    try $0.previewImport(path: path, connector: connector, policy: policy, force: force)
                }
                guard !Task.isCancelled else { return }
                self.preview = preview
                plan = preview.view()
            } catch {
                guard !Task.isCancelled else { return }
                preview = nil
                plan = nil
                problem = describe(error)
            }
        }
    }

    func apply() {
        guard let preview else { return }
        isWorking = true
        Task {
            defer { isWorking = false }
            do {
                summary = try await library.app.call { try $0.applyImport(preview: preview) }
                await library.reload()
            } catch {
                problem = describe(error)
            }
        }
    }
}

/// Which words an export takes.
enum ExportChoice: Hashable, CaseIterable {
    case new
    case all
    case selected
    case filter
}

/// Export (mockup 11): choose where and what, see how many, write the file.
@MainActor
@Observable
final class ExportModel {
    let library: LibraryModel
    /// The words selected in the library when the sheet opened.
    let selected: [Int64]
    /// The library's current view, when it can be exported as such.
    let filter: LibraryModel.ExportFilter?

    var connector: String {
        didSet {
            if connector != oldValue {
                if !destinationChosen { destination = Self.suggestedFile(for: connector) }
                refresh()
            }
        }
    }
    var choice: ExportChoice {
        didSet { if choice != oldValue { refresh() } }
    }
    var includeNeedsReview = false {
        didSet { if includeNeedsReview != oldValue { refresh() } }
    }
    /// Anki only: the deck to import into.
    var deck = "" {
        didSet { if deck != oldValue { refresh(after: .milliseconds(300)) } }
    }
    private(set) var destination: URL
    private(set) var counts: [ExportChoice: Int] = [:]
    private(set) var preview: ExportPreview?
    private(set) var plan: ExportPlanView?
    private(set) var problem: String?
    private(set) var isWorking = false
    private(set) var summary: TransferSummary?

    /// Chosen in the save panel, which already asked before replacing a file.
    @ObservationIgnored private var destinationChosen = false
    @ObservationIgnored private var previewing: Task<Void, Never>?
    /// Counts previews, so only the newest one clears `isWorking`.
    @ObservationIgnored private var generation = 0

    init(library: LibraryModel) {
        self.library = library
        selected = library.selectedItems.map(\.id)
        filter = library.exportFilter
        let first = library.connectors.first(where: \.canExport)?.id ?? "anki"
        connector = first
        choice = library.selectedItems.count > 1 ? .selected : .new
        destination = Self.suggestedFile(for: first)
    }

    /// The first count; called when the sheet appears.
    func start() {
        if plan == nil && !isWorking { refresh() }
    }

    var formats: [ConnectorView] { library.connectors.filter(\.canExport) }

    var choices: [ExportChoice] {
        ExportChoice.allCases.filter { choice in
            switch choice {
            case .selected: !selected.isEmpty
            case .filter: filter != nil
            default: true
            }
        }
    }

    var formatName: String { formats.first { $0.id == connector }?.name ?? connector }
    var count: Int { plan?.itemIds.count ?? 0 }
    var canExport: Bool { count > 0 && !isWorking && problem == nil && summary == nil }

    /// Exporting would replace a file nobody has agreed to replace: the
    /// suggested destination, already there from an earlier export.
    var needsReplaceConfirmation: Bool { plan?.replacesExisting == true && !destinationChosen }

    private static func suggestedFile(for connector: String) -> URL {
        let folder = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser
        return folder.appending(path: "shouci-\(connector).txt")
    }

    func chooseDestination() {
        let panel = NSSavePanel()
        panel.message = "Choose where to save the \(formatName) file."
        panel.nameFieldStringValue = destination.lastPathComponent
        panel.directoryURL = destination.deletingLastPathComponent()
        panel.allowedContentTypes = [.plainText]
        panel.canCreateDirectories = true
        if panel.runModal() == .OK, let url = panel.url {
            destination = url
            destinationChosen = true
            refresh()
        }
    }

    private func request(for choice: ExportChoice) -> ExportRequest {
        let deck = connector == "anki" && !self.deck.trimmingCharacters(in: .whitespaces).isEmpty
            ? self.deck.trimmingCharacters(in: .whitespaces) : nil
        switch choice {
        case .new:
            return ExportRequest(scope: .new, filter: LibraryFilter(), includeNeedsReview: includeNeedsReview, deck: deck)
        case .all:
            return ExportRequest(scope: .all, filter: LibraryFilter(), includeNeedsReview: includeNeedsReview, deck: deck)
        case .selected:
            return ExportRequest(
                scope: .selected(selected), filter: LibraryFilter(view: .all),
                includeNeedsReview: includeNeedsReview, deck: deck)
        case .filter:
            return ExportRequest(
                scope: .all, filter: filter?.filter ?? LibraryFilter(),
                includeNeedsReview: includeNeedsReview, deck: deck)
        }
    }

    /// Previews every choice (for its count) and keeps the chosen one.
    private func refresh(after delay: Duration = .zero) {
        let path = destination.path
        let connector = self.connector
        let chosen = choice
        let requests = choices.map { ($0, request(for: $0)) }
        previewing?.cancel()
        isWorking = true
        generation += 1
        let mine = generation
        previewing = Task {
            defer { if generation == mine { isWorking = false } }
            if delay > .zero { try? await Task.sleep(for: delay) }
            guard !Task.isCancelled else { return }
            // One call per preview, so a newer refresh stops this one between
            // them instead of after all of them.
            var results: [Outcome] = []
            for (choice, request) in requests {
                do {
                    let preview = try await library.app.call {
                        try $0.previewExport(path: path, connector: connector, request: request)
                    }
                    results.append(Outcome(choice: choice, preview: preview, problem: nil))
                } catch {
                    guard !Task.isCancelled else { return }
                    results.append(Outcome(choice: choice, preview: nil, problem: describe(error)))
                }
            }
            guard !Task.isCancelled else { return }
            var counts: [ExportChoice: Int] = [:]
            for result in results {
                if let preview = result.preview { counts[result.choice] = preview.view().itemIds.count }
            }
            self.counts = counts
            let chosenOutcome = results.first { $0.choice == chosen }
            preview = chosenOutcome?.preview
            plan = chosenOutcome?.preview?.view()
            problem = chosenOutcome?.problem
        }
    }

    private struct Outcome: Sendable {
        let choice: ExportChoice
        let preview: ExportPreview?
        let problem: String?
    }

    func apply() {
        guard let preview else { return }
        isWorking = true
        Task {
            defer { isWorking = false }
            do {
                summary = try await library.app.call { try $0.applyExport(preview: preview) }
                await library.reload()
            } catch {
                problem = describe(error)
            }
        }
    }

    func showInFinder() {
        NSWorkspace.shared.activateFileViewerSelecting([destination])
    }
}
