import ShouciCore
import SwiftUI

/// Library views, smart collections, collections, and tags, with the
/// dictionary's state below (mockup 01).
struct LibrarySidebar: View {
    @Bindable var model: LibraryModel

    var body: some View {
        List(selection: scope) {
            Section("Library") {
                row(.all, "All Vocabulary", icon: "list.bullet.rectangle")
                row(.needsReview, "Needs Review", icon: "exclamationmark.triangle", tint: Palette.warn)
                row(.noCollection, "Not in a Collection", icon: "tray")
                row(.recent, "Recently Added", icon: "clock", counted: false)
                row(.archived, "Archived", icon: "archivebox")
                row(.trash, "Trash", icon: "trash")
            }
            Section {
                ForEach(model.smartCollections) { smart in
                    Label(smart.name, systemImage: "gearshape")
                        .badge(smart.itemIds.count)
                        .tag(LibraryModel.Scope.smart(smart.name))
                        .contextMenu {
                            Button("Rename…") {
                                model.naming = .init(purpose: .renameSmartCollection(smart.name), text: smart.name)
                            }
                            Button("Delete Smart Collection") { model.deleteSmartCollection(smart.name) }
                        }
                }
            } header: {
                header(
                    "Smart Collections", adding: "New Smart Collection",
                    help: "New Smart Collection: filter All Vocabulary, then save the filter",
                    action: model.newSmartCollection)
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
                            mergeMenu(.collection, from: collection.name, into: model.collections)
                            Button("Delete Collection") { model.deleteCollection(collection.name) }
                        }
                }
            } header: {
                header("Collections", adding: "New Collection") {
                    model.naming = .init(purpose: .newCollection(then: []), text: "")
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
                                mergeMenu(.tag, from: tag.name, into: model.tags)
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

    /// A section's title, with a + that adds to it.
    private func header(
        _ title: String, adding: String, help: String? = nil, action: @escaping () -> Void
    ) -> some View {
        HStack {
            Text(title)
            Spacer()
            Button(action: action) {
                Image(systemName: "plus")
                    .frame(width: 20, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(help ?? adding)
            .accessibilityLabel(adding)
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
            SidebarIcon(symbol: icon, tint: tint ?? Palette.accent)
        }
        .badge(counted ? model.count(of: scope) : 0)
        .tag(scope)
    }

    /// Fold this tag or collection into another: `HSK1` into `HSK 1`.
    @ViewBuilder
    private func mergeMenu(_ kind: GroupKind, from name: String, into groups: [GroupView]) -> some View {
        let others = groups.filter { $0.name != name }
        if !others.isEmpty {
            Menu("Merge Into") {
                ForEach(others) { other in
                    Button(other.name) { model.merging = .init(kind: kind, from: name, into: other.name) }
                }
            }
        }
    }
}

/// A Library view's icon in its color, and white on the selection, where
/// indigo on indigo would vanish.
private struct SidebarIcon: View {
    let symbol: String
    let tint: Color
    @Environment(\.backgroundProminence) private var prominence

    var body: some View {
        Image(systemName: symbol)
            .foregroundStyle(prominence == .increased ? AnyShapeStyle(.primary) : AnyShapeStyle(tint))
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
        .font(Typography.control)
        .foregroundStyle(Palette.secondary)
        .lineLimit(1)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Rows scroll under the footer; without a background they show
        // through its text.
        .background(.bar)
        .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 1) }
    }
}
