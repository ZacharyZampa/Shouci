import AppKit
import ShouciCore
import SwiftUI

/// Settings › Dictionaries (mockup 13): which dictionaries search uses, in
/// what order, and whether they are current.
struct DictionarySettings: View {
    let app: AppModel
    @State private var dictionaries: [DictionaryView] = []
    @State private var loaded = false
    @State private var problem: String?

    var body: some View {
        Form {
            Section {
                Text("Choose which dictionaries Shouci searches and shows. Your saved words stay as you edited them, whichever dictionary you view.")
                    .foregroundStyle(.secondary)
            }
            Section("Lookup dictionaries") {
                if dictionaries.isEmpty {
                    Text(loaded ? "No dictionary is installed yet." : "Loading…")
                        .foregroundStyle(.secondary)
                }
                ForEach(Array(dictionaries.enumerated()), id: \.element.id) { index, dictionary in
                    row(dictionary, at: index)
                }
            }
            Section("Ranking") {
                LabeledContent {
                    EmptyView()
                } label: {
                    Text("Word frequency and HSK level")
                    Text("Common words come first in results: OpenSubtitles frequency, then HSK 3.0 level. Built into CC-CEDICT, not shown as a dictionary.")
                }
            }
            Section {
                HStack {
                    status
                    Spacer()
                    Button("Check Now", action: app.loadDictionaries).disabled(isLoading)
                }
            } footer: {
                Text("Shouci looks for a new CC-CEDICT once a month and keeps the current copy if the download fails.")
                    .foregroundStyle(.secondary)
            }
            if let problem {
                Text(problem).foregroundStyle(Palette.warn)
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fixedSize(horizontal: false, vertical: true)
        .task(id: app.dictionary) { await load() }
    }

    private func row(_ dictionary: DictionaryView, at index: Int) -> some View {
        let enabledCount = dictionaries.filter(\.enabled).count
        return HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    Text(dictionary.name)
                    if index == 0 && dictionary.enabled && dictionaries.count > 1 { Chip(text: "Shown first") }
                }
                Text("\(dictionary.entries.formatted()) entries · \(dictionary.license)")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if dictionaries.count > 1 {
                Button { move(dictionary, by: -1) } label: { Image(systemName: "chevron.up") }
                    .disabled(index == 0 || !dictionary.enabled)
                    .accessibilityLabel("Move \(dictionary.name) up")
                Button { move(dictionary, by: 1) } label: { Image(systemName: "chevron.down") }
                    .disabled(index >= enabledCount - 1 || !dictionary.enabled)
                    .accessibilityLabel("Move \(dictionary.name) down")
            }
            Toggle(dictionary.name, isOn: Binding(get: { dictionary.enabled }, set: { setEnabled(dictionary, $0) }))
                .labelsHidden()
                .toggleStyle(.switch)
                .disabled(dictionary.enabled && enabledCount == 1)
                .help(dictionary.enabled && enabledCount == 1 ? "At least one dictionary stays on." : "")
        }
    }

    private var isLoading: Bool {
        switch app.dictionary {
        case .starting, .loading, .ready(updating: .some): true
        default: false
        }
    }

    @ViewBuilder
    private var status: some View {
        switch app.dictionary {
        case .ready(nil):
            Label("Ready", systemImage: "checkmark.circle").foregroundStyle(Palette.ok)
        case .ready(let updating?):
            Label(updating, systemImage: "arrow.down.circle")
        case .starting, .loading:
            Label("Loading the dictionary…", systemImage: "arrow.down.circle")
        case .failed(let message):
            Label(message, systemImage: "exclamationmark.triangle").foregroundStyle(Palette.warn)
        }
    }

    private func load() async {
        guard app.core != nil else { return }
        do {
            dictionaries = try await app.call { try $0.dictionaries() }
            loaded = true
        } catch {
            problem = describe(error)
        }
    }

    private func setEnabled(_ dictionary: DictionaryView, _ enabled: Bool) {
        var ids = dictionaries.filter(\.enabled).map(\.id)
        if enabled { ids.append(dictionary.id) } else { ids.removeAll { $0 == dictionary.id } }
        apply(ids)
    }

    private func move(_ dictionary: DictionaryView, by offset: Int) {
        var ids = dictionaries.filter(\.enabled).map(\.id)
        guard let index = ids.firstIndex(of: dictionary.id), ids.indices.contains(index + offset) else { return }
        ids.swapAt(index, index + offset)
        apply(ids)
    }

    private func apply(_ ids: [String]) {
        Task {
            do {
                dictionaries = try await app.call { try $0.setEnabledDictionaries(ids: ids) }
                problem = nil
            } catch {
                problem = describe(error)
            }
        }
    }
}

/// Settings › Data: where the library lives, and moving words in and out.
struct DataSettings: View {
    let app: AppModel
    let openImport: () -> Void
    let openExport: () -> Void
    @State private var bringOver: String?

    var body: some View {
        Form {
            Section("Your library") {
                if let config = app.core?.config() {
                    location("Words", path: config.dataDir)
                    location("Dictionaries", path: config.dictionariesDir)
                } else {
                    Text("Opening your library…").foregroundStyle(.secondary)
                }
            }
            Section("Moving words") {
                LabeledContent {
                    HStack {
                        Button("Import…", action: openImport)
                        Button("Export…", action: openExport)
                    }
                } label: {
                    Text("Pleco and Anki")
                    Text("Plain-text files that Pleco and Anki read and write.")
                }
                LabeledContent {
                    Button("Choose File…", action: chooseOldLibrary)
                } label: {
                    Text("An older Shouci library")
                    Text(bringOver ?? "Brings words over from a proof-of-concept user.db. Only new words are added.")
                }
            }
            Section {
                Text("Your words stay on this Mac. Shouci goes online only to download dictionaries.")
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func location(_ label: String, path: String) -> some View {
        LabeledContent {
            Button("Show in Finder") {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
            }
        } label: {
            Text(label)
            Text((path as NSString).abbreviatingWithTildeInPath)
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }

    private func chooseOldLibrary() {
        let panel = NSOpenPanel()
        panel.message = "Choose the user.db of an older Shouci (pleco-companion) library."
        panel.prompt = "Bring Words Over"
        panel.directoryURL = FileManager.default.homeDirectoryForCurrentUser
            .appending(path: "Library/Application Support/pleco-companion")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let path = url.path
        Task {
            do {
                let result = try await app.call { try $0.importPoc(legacyDb: path) }
                var parts = ["\(result.itemsAdded) words added", "\(result.itemsAlreadySaved) already here"]
                if !result.skipped.isEmpty { parts.append("\(result.skipped.count) could not come over") }
                bringOver = parts.joined(separator: ", ") + "."
            } catch {
                bringOver = describe(error)
            }
        }
    }
}
