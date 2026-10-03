import Foundation
import Observation
import ShouciCore

/// Where the dictionaries stand, worded for people.
enum DictionaryState: Equatable {
    case starting
    case loading(String)
    /// Search works. `updating` describes a refresh running in the background.
    case ready(updating: String?)
    case failed(String)
}

/// The app's one `Core`, opened and loaded in the background. Every call
/// into the core goes through `call`, off the main thread.
@MainActor
@Observable
final class AppModel {
    private(set) var core: Core?
    /// Why the library could not be opened. Nothing works without it.
    private(set) var openFailure: String?
    private(set) var dictionary: DictionaryState = .starting
    /// Things to tell the user once: words brought over from the proof of
    /// concept, a monthly refresh that failed.
    private(set) var notices: [String] = []

    @ObservationIgnored private var seenNotes: Set<String> = []
    @ObservationIgnored private var polling: Task<Void, Never>?

    var isSearchable: Bool {
        if case .ready = dictionary { core != nil } else { false }
    }

    func start() {
        Task {
            do {
                let core = try await Task.detached(priority: .userInitiated) {
                    try Core.open(config: defaultConfig())
                }.value
                self.core = core
                core.startupNotes().forEach(post)
                loadDictionaries()
            } catch {
                openFailure = describe(error)
                dictionary = .failed(describe(error))
            }
        }
    }

    /// Opens what is installed, then downloads or refreshes in the
    /// background. Also the retry after a failure.
    func loadDictionaries() {
        guard let core else { return }
        dictionary = .loading("Preparing the dictionary…")
        Task.detached(priority: .utility) { try? core.loadDictionaries() }
        polling?.cancel()
        polling = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, let core = self.core else { return }
                if self.apply(core.dictionaryStatus()) { return }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
    }

    func call<T: Sendable>(_ work: @escaping @Sendable (Core) throws -> T) async throws -> T {
        guard let core else {
            throw ShouciError.Failed(kind: .unavailable, message: "Shouci is still opening your library.")
        }
        return try await Task.detached(priority: .userInitiated) { try work(core) }.value
    }

    func post(_ notice: String) {
        notices.append(notice)
    }

    func dismissNotice() {
        if !notices.isEmpty { notices.removeFirst() }
    }

    /// Shows a status; true once it will not change without another load.
    private func apply(_ status: DictionaryStatus) -> Bool {
        switch status {
        case .notLoaded:
            dictionary = .loading("Preparing the dictionary…")
            return false
        case .loading(let stage, let name):
            dictionary = .loading(Self.progress(stage, name: name))
            return false
        case .ready(_, let notes, let updating):
            for note in notes where seenNotes.insert(note).inserted {
                post(note)
            }
            dictionary = .ready(updating: updating.map { _ in "Updating the dictionary…" })
            return updating == nil
        case .failed(let message):
            dictionary = .failed(message)
            return true
        }
    }

    private static func progress(_ stage: LoadingStage, name: String?) -> String {
        let name = name ?? "the dictionary"
        switch stage {
        case .checking: return "Checking the dictionary…"
        case .waiting: return "Waiting for another Shouci to finish \(name)…"
        case .downloading: return "Downloading \(name). This happens once and needs an internet connection."
        case .building: return "Building \(name)…"
        case .opening: return "Opening the dictionary…"
        }
    }
}
