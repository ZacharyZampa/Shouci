import Foundation
import ShouciCore

// The filter panel and smart collections. The filter narrows the view
// shown: its conditions go to the core, which answers with the words that
// match (`matchingIds`), since only the core knows HSK levels and frequency.
// Saving the filter keeps it as a smart collection, listed in the sidebar:
// the view's own condition (a collection, Needs Review) is saved with it.
// Showing a smart collection puts its whole filter in the panel, to change
// and save again. Creating, changing, renaming, and deleting one can be
// taken back with ⌘Z.

extension LibraryModel {
    func smartCollection(named name: String) -> SmartCollectionView? {
        smartCollections.first { $0.name == name }
    }

    /// The smart collection shown, if one is.
    var shownSmart: SmartCollectionView? {
        guard case .smart(let name) = scope else { return nil }
        return smartCollection(named: name)
    }

    /// Whether the filter narrows a Library view, collection, or tag.
    var isFiltering: Bool {
        if case .smart = scope { return false }
        return filter.hasConditions
    }

    /// The smart collection shown has changes in the panel not yet saved.
    var hasUnsavedSmartChanges: Bool {
        shownSmart.map { !filter.sameConditions(as: $0.filter) } ?? false
    }

    /// What the core is asked for: the filter over every word the view
    /// could show, or nothing when no filter narrows it.
    var filterQuery: LibraryFilter? {
        if case .smart = scope { return filter }
        guard filter.hasConditions else { return nil }
        var query = filter
        query.view = scope == .trash ? .trash : .all
        return query
    }

    /// The view and its filter as one filter: what a smart collection saved
    /// from here keeps.
    var savableFilter: LibraryFilter {
        var saved = filter
        switch scope {
        case .all:
            saved.view = .active
        case .needsReview:
            saved.view = .active
            saved.verification = .needsReview
        case .noCollection:
            saved.view = .active
            saved.noCollection = true
        case .recent:
            saved.view = .active
            saved.addedWithinDays = min(saved.addedWithinDays ?? 7, 7)
        case .archived:
            saved.view = .archived
        case .trash:
            saved.view = .trash
        case .collection(let name):
            saved.view = .all
            saved.collection = name
        case .tag(let name):
            saved.view = .all
            if !saved.tags.contains(name) { saved.tags.append(name) }
        case .smart:
            break
        }
        return saved
    }

    // MARK: Filtering

    /// A new view starts unfiltered; a smart collection starts with its own
    /// filter and the words the last reload found for it.
    func adoptFilter() {
        filtering?.cancel()
        if let smart = shownSmart {
            filter = smart.filter
            filtered = Set(smart.itemIds)
        } else {
            filter = LibraryFilter()
            filtered = nil
        }
    }

    /// Changes the panel's conditions and asks the core which words match.
    func setFilter(_ filter: LibraryFilter) {
        guard filter != self.filter else { return }
        self.filter = filter
        filtering?.cancel()
        guard let query = filterQuery else {
            filtered = nil
            refreshList()
            return
        }
        filtering = Task {
            do {
                let ids = try await app.call { try $0.matchingIds(filter: query) }
                // A newer change asks again.
                guard !Task.isCancelled, query == filterQuery else { return }
                filtered = Set(ids)
                refreshList()
                if !isSearching {
                    let listed = Set(order)
                    selection = selection.filter(listed.contains)
                }
            } catch {
                guard !Task.isCancelled else { return }
                problem = describe(error)
            }
        }
    }

    /// After a reload: the words the filter matches now. A smart collection
    /// whose filter wasn't changed here takes its new filter (a tag renamed,
    /// `shouci smart save`) along with its words. `matched` is nil when the
    /// filter changed during the reload: that change asks again.
    func refilter(after previous: [SmartCollectionView], matched: [Int64]?) {
        if let saved = shownSmart {
            let unchanged = previous.first { $0.name == saved.name }.map { filter.sameConditions(as: $0.filter) } ?? true
            if unchanged || filter.sameConditions(as: saved.filter) {
                filter = saved.filter
                filtered = Set(saved.itemIds)
                return
            }
        }
        if let matched { filtered = Set(matched) }
    }

    /// Takes every condition off, keeping which words a smart collection
    /// shows (active, archived, or both).
    func clearFilter() {
        setFilter(LibraryFilter(view: filter.view))
    }

    // MARK: Smart collections

    /// Names the filter, to keep it as a smart collection.
    func saveAsSmartCollection() {
        showingFilter = false
        naming = Naming(purpose: .newSmartCollection(savableFilter), text: "")
    }

    /// Starts a smart collection from All Vocabulary, with the panel open.
    func newSmartCollection() {
        if scope != .all { scope = .all }
        showingFilter = true
    }

    // Like every library change, these go through `record` (LibraryUndo.swift):
    // the core's snapshots hold every smart collection, and ⌘Z puts back
    // the ones a change made, changed, or deleted.

    func createSmartCollection(_ name: String, filter: LibraryFilter) {
        let created = record("New Smart Collection", ids: []) {
            _ = try $0.createSmartCollection(name: name, filter: filter)
        }
        Task { if await created.value { scope = .smart(name) } }
    }

    /// Saves the panel's conditions as the shown smart collection's filter.
    func saveSmartChanges() {
        guard let name = shownSmart?.name else { return }
        showingFilter = false
        let filter = self.filter
        record("Change Smart Collection", ids: []) { _ = try $0.updateSmartCollection(name: name, filter: filter) }
    }

    /// Puts the shown smart collection's saved filter back in the panel.
    func revertSmartChanges() {
        guard let smart = shownSmart else { return }
        filtering?.cancel()
        filter = smart.filter
        filtered = Set(smart.itemIds)
        refreshList()
    }

    func renameSmartCollection(_ old: String, to name: String) {
        guard name != old else { return }
        // Asked before the change: reloading leaves a view whose name is
        // gone. Conditions changed in the panel but not saved stay there.
        let showing = scope == .smart(old)
        let conditions = filter
        let renamed = record("Rename Smart Collection", ids: []) { try $0.renameSmartCollection(from: old, to: name) }
        Task {
            if await renamed.value, showing {
                scope = .smart(name)
                setFilter(conditions)
            }
        }
    }

    func deleteSmartCollection(_ name: String) {
        record("Delete Smart Collection", ids: []) { try $0.deleteSmartCollection(name: name) }
    }
}

// What a filter asks for is the core's to say, so the window's chips and
// `shouci smart` word it the same way.
extension LibraryFilter {
    /// Whether anything narrows the words, besides which of them (active,
    /// archived) a smart collection shows.
    var hasConditions: Bool { filterHasConditions(filter: self) }

    /// The conditions, in the order the panel lists them, each with the
    /// filter that taking it off leaves.
    var conditions: [FilterConditionView] { filterConditions(filter: self) }

    /// The same conditions, whatever order their values were added in.
    func sameConditions(as other: LibraryFilter) -> Bool { ShouciCore.sameConditions(a: self, b: other) }
}
