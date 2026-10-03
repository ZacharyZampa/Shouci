import ShouciCore
import SwiftUI

/// Library views, collections, and tags, with the dictionary's state below
/// (mockup 01).
struct LibrarySidebar: View {
    @Bindable var model: LibraryModel

    var body: some View {
        List(selection: scope) {
            Section("Library") {
                row(.all, "All Vocabulary", icon: "list.bullet.rectangle")
                row(.needsReview, "Needs Review", icon: "exclamationmark.triangle", tint: Palette.warn)
                row(.recent, "Recently Added", icon: "clock", counted: false)
                row(.archived, "Archived", icon: "archivebox")
                row(.trash, "Trash", icon: "trash")
            }
            Section {
                ForEach(model.collections) { collection in
                    Label(collection.name, systemImage: "folder")
                        .badge(Int(collection.count))
                        .tag(LibraryModel.Scope.collection(collection.name))
                        .contextMenu {
                            Button("Rename…") {
                                model.naming = .init(purpose: .renameCollection(collection.name), text: collection.name)
                            }
                            Button("Delete Collection") { model.deleteCollection(collection.name) }
                        }
                }
            } header: {
                HStack {
                    Text("Collections")
                    Spacer()
                    Button {
                        model.naming = .init(purpose: .newCollection(then: []), text: "")
                    } label: {
                        Image(systemName: "plus")
                    }
                    .buttonStyle(.plain)
                    .help("New Collection")
                    .accessibilityLabel("New Collection")
                }
            }
            if !model.tags.isEmpty {
                Section("Tags") {
                    ForEach(model.tags) { tag in
                        Label(tag.name, systemImage: "tag")
                            .badge(Int(tag.count))
                            .tag(LibraryModel.Scope.tag(tag.name))
                            .contextMenu {
                                Button("Rename…") {
                                    model.naming = .init(purpose: .renameTag(tag.name), text: tag.name)
                                }
                                Button("Make a Collection") { model.makeCollection(fromTag: tag.name) }
                                Button("Delete Tag") { model.deleteTag(tag.name) }
                            }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            DictionaryFooter(app: model.app)
        }
    }

    private var scope: Binding<LibraryModel.Scope?> {
        Binding(get: { model.scope }, set: { if let scope = $0 { model.scope = scope } })
    }

    private func row(
        _ scope: LibraryModel.Scope, _ title: String, icon: String, tint: Color? = nil, counted: Bool = true
    ) -> some View {
        Label {
            Text(title)
        } icon: {
            Image(systemName: icon).foregroundStyle(tint ?? Palette.accent)
        }
        .badge(counted ? model.items(in: scope).count : 0)
        .tag(scope)
    }
}

/// Where the dictionary stands, at the foot of the sidebar.
private struct DictionaryFooter: View {
    let app: AppModel

    var body: some View {
        HStack(spacing: 8) {
            switch app.dictionary {
            case .ready(nil):
                Image(systemName: "checkmark.circle").foregroundStyle(Palette.ok)
                Text("Dictionary ready")
            case .ready(let updating?):
                ProgressView().controlSize(.mini)
                Text(updating)
            case .starting:
                ProgressView().controlSize(.mini)
                Text("Opening your library…")
            case .loading:
                ProgressView().controlSize(.mini)
                Text("Loading the dictionary…")
            case .failed:
                Image(systemName: "exclamationmark.triangle").foregroundStyle(Palette.warn)
                Text("Dictionary unavailable")
                Spacer()
                Button("Retry", action: app.loadDictionaries).controlSize(.small)
            }
        }
        .font(.system(size: 12))
        .foregroundStyle(Palette.secondary)
        .lineLimit(1)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 1) }
    }
}
