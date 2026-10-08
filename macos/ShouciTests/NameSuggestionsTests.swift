import ShouciCore
import Testing

/// What the tag and collection field offers as a name is typed.
struct NameSuggestionsTests {
    private let known = [GroupView(name: "drill", count: 3), GroupView(name: "Food", count: 9), GroupView(name: "redo", count: 1)]

    @Test func beforeTypingRecentNamesLeadThenTheMostUsed() {
        let suggestions = NameSuggestions(query: "", known: known, present: [], recent: ["redo"])
        #expect(suggestions.rows.map(\.name) == ["redo", "Food", "drill"])
        #expect(suggestions.preferred == 0, "↵ picks the one used last")
    }

    @Test func namesStartingWithWhatWasTypedComeFirstThenCreatingIt() {
        let suggestions = NameSuggestions(query: "d", known: known, present: [])
        #expect(suggestions.rows == [
            .existing("drill", count: 3), .existing("Food", count: 9), .existing("redo", count: 1), .create("d"),
        ])
        #expect(suggestions.preferred == 0)
    }

    @Test func anExactMatchIsPickedAndNotCreatedAgainWhateverItsCase() {
        let suggestions = NameSuggestions(query: "food", known: known, present: [])
        #expect(suggestions.rows == [.existing("Food", count: 9)])
        #expect(suggestions.preferred == 0)
    }

    @Test func aNameTheWordHasIsShownButNotPicked() {
        let suggestions = NameSuggestions(query: "drill", known: known, present: ["Drill"])
        #expect(suggestions.rows == [.added("drill")])
        #expect(suggestions.preferred == nil)
    }
}
