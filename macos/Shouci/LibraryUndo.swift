import Foundation
import ShouciCore

// ⌘Z in the library window. A change goes through `record`, which reads the
// words it touches before and after it, with every name and smart
// collection (the core's `snapshot`); undoing restores the first reading
// and redoing the second (`restore`). The core refuses, changing nothing,
// when a word or smart collection changed in the meantime, so an undo never
// overwrites what `shouci` or quick search did since. The registrations go
// to the library window's own undo manager, so the Edit menu names them.
//
// Each direction registers the other while the undo manager runs it: a
// registration made after the core call returns, outside undo or redo,
// would land on the undo stack and empty the redo stack.
//
// A word saved new is missing from the first reading, so undoing deletes it
// for good and redoing puts the same word back; one brought back from the
// trash goes back there, and one saved already loses what the save gave it
// (tags, collections, Needs Review). Emptying the Trash, deleting for good,
// and imports can't be taken back.

extension LibraryModel {
    /// Runs a change ⌘Z can take back, after the changes asked for before
    /// it, then reloads. The task's value is false when it failed (and says
    /// why).
    @discardableResult
    func record(_ name: String, ids: [Int64], _ work: @escaping @Sendable (Core) throws -> Void) -> Task<Bool, Never> {
        record(name, reading: { _ in ids }, work)
    }

    /// As `record`, reading which words the change touches as it runs: the
    /// words of a tag or collection, which the list may not show yet.
    @discardableResult
    func record(
        _ name: String, reading touched: @escaping @Sendable (Core) throws -> [Int64],
        _ work: @escaping @Sendable (Core) throws -> Void
    ) -> Task<Bool, Never> {
        queued(failed: false) { [self] in
            let (before, after) = try await app.call { core in
                let ids = try touched(core)
                let before = try core.snapshot(ids: ids)
                try work(core)
                return (before, try core.snapshot(ids: ids))
            }
            registerRestore(name, to: before, from: after)
            return true
        }
    }

    /// Saves a word ⌘Z can take back, after the changes asked for before
    /// it. `known` reads the saved word the save would change, if any. The
    /// task's value is the word's id, or nil when saving failed.
    func recordAdd(
        _ name: String, known: @escaping @Sendable (Core) throws -> [Int64] = { _ in [] },
        _ save: @escaping @Sendable (Core) throws -> SaveResult
    ) -> Task<Int64?, Never> {
        queued(failed: nil) { [self] in
            let (before, saved, after) = try await app.call { core in
                let ids = try known(core)
                let before = try core.snapshot(ids: ids)
                let saved = try save(core)
                return (before, saved, try core.snapshot(ids: ids + [saved.item.id]))
            }
            if saved.canUndo(before: before, after: after) { registerRestore(name, to: before, from: after) }
            return saved.item.id
        }
    }

    /// Registers putting the words back as `target` has them, now that they
    /// are as `current` has them.
    private func registerRestore(_ name: String, to target: LibrarySnapshot, from current: LibrarySnapshot) {
        guard let undoManager = undoManager() else { return }
        undoManager.registerUndo(withTarget: self) { model in
            MainActor.assumeIsolated {
                model.registerRestore(name, to: current, from: target)
                model.enqueue { try $0.restore(target: target, current: current) }
            }
        }
        undoManager.setActionName(name)
    }
}

extension SaveResult {
    /// Whether ⌘Z (or quick search's Undo) takes this save back, given the
    /// library before and after it, each read with the word the save would
    /// change: a word saved new, or a saved word the save changed (brought
    /// back from the trash, given a reading, tags, or collections). A save
    /// that changed nothing has nothing to take back, and a word saved by
    /// someone else just before isn't this save's.
    func canUndo(before: LibrarySnapshot, after: LibrarySnapshot) -> Bool {
        guard let was = before.words.first(where: { $0.id == item.id }) else { return outcome == .inserted }
        // Every change to a word, its tags, or its collections raises its
        // revision.
        let now = after.words.first { $0.id == item.id }
        return now?.rev != was.rev
    }
}
