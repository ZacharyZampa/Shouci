import ShouciCore
import SwiftUI

/// The list header's Filter button, which opens the filter panel.
struct FilterButton: View {
    @Bindable var model: LibraryModel

    var body: some View {
        let count = model.filter.conditions.count
        Button {
            model.showingFilter.toggle()
        } label: {
            HStack(spacing: 4) {
                Image(systemName: count > 0 ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
                Text(count > 0 ? "Filter · \(count)" : "Filter")
            }
            .foregroundStyle(count > 0 ? Palette.accent : Palette.text)
        }
        .controlSize(.small)
        .help("Filter by HSK level, frequency, status, tags, collections, and when words were added")
        .popover(isPresented: $model.showingFilter, arrowEdge: .bottom) {
            FilterPanel(model: model)
        }
    }
}

/// What the words must be (mockup A): within a row any value will do, and
/// every row must hold. Changes show in the list as they are made.
struct FilterPanel: View {
    @Bindable var model: LibraryModel

    /// As the core offers them: 0 is no level, 7 the advanced band.
    private static let levels = hskLevels()
    private static let periods: [UInt32?] = [nil, 7, 30, 90, 365]

    private var filter: LibraryFilter { model.filter }

    /// From the foot of the window's toolbar to the top of the panel: the
    /// list's header, where the Filter button sits, and the popover's arrow.
    private static let belowToolbar: CGFloat = 56

    @State private var footerHeight: CGFloat = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            conditions
                .padding([.horizontal, .top], 16)
                .padding(.bottom, 14)
                .scrollsBeyond(ScreenRoom.current.belowToolbar - Self.belowToolbar - footerHeight - ScreenRoom.margin)
            Divider().padding(.horizontal, 16)
            footer
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 16)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { footerHeight = $0 }
        }
        .font(Typography.control)
        .frame(width: 360)
    }

    private var conditions: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                Text("Filter").font(Typography.emphasis)
                Spacer()
                Text(model.shownCount == 1 ? "1 word" : "\(model.shownCount) words")
                    .foregroundStyle(Palette.tertiary)
            }
            row("HSK Level") {
                ForEach(Self.levels, id: \.level) { choice in
                    ToggleChip(title: choice.label, isOn: filter.hskLevels.contains(choice.level)) {
                        change { $0.hskLevels = toggled($0.hskLevels, choice.level).sorted() }
                    }
                }
            }
            row("Frequency") {
                ForEach(LibraryList.bands) { band in
                    ToggleChip(title: band.shortLabel, isOn: filter.frequencyBands.contains(band.band)) {
                        change { filter in
                            let chosen = toggled(filter.frequencyBands, band.band)
                            // Kept in the bands' order, most common first.
                            filter.frequencyBands = LibraryList.bands.map(\.band).filter(chosen.contains)
                        }
                    }
                    .help(band.label)
                }
            }
            row("Status") {
                ForEach([Verification.needsReview, .confirmed], id: \.self) { verification in
                    ToggleChip(
                        title: verification == .needsReview ? "Needs Review" : "Confirmed",
                        isOn: filter.verification == verification
                    ) {
                        change { $0.verification = $0.verification == verification ? nil : verification }
                    }
                }
            }
            row("Added") {
                ForEach(Self.periods, id: \.self) { days in
                    ToggleChip(title: days.map(Self.periodTitle) ?? "Any Time", isOn: filter.addedWithinDays == days) {
                        change { $0.addedWithinDays = days }
                    }
                }
            }
            section("Tags") {
                NameRow(
                    title: "With", kind: .tag, names: filter.anyTags, known: model.tags,
                    add: { name in change { $0.addAnyTag(name) } },
                    remove: { name in change { $0.anyTags.removeAll { $0 == name } } })
                NameRow(
                    title: "Without", kind: .tag, names: filter.withoutTags, known: model.tags,
                    add: { name in change { $0.addWithoutTag(name) } },
                    remove: { name in change { $0.withoutTags.removeAll { $0 == name } } })
            }
            section("Collections") {
                NameRow(
                    title: "In", kind: .collection, names: filter.anyCollections, known: model.collections,
                    add: { name in change { $0.addAnyCollection(name) } },
                    remove: { name in change { $0.anyCollections.removeAll { $0 == name } } })
                NameRow(
                    title: "Not in", kind: .collection, names: filter.withoutCollections, known: model.collections,
                    add: { name in change { $0.addWithoutCollection(name) } },
                    remove: { name in change { $0.withoutCollections.removeAll { $0 == name } } })
                Toggle(
                    "In no collection",
                    isOn: Binding(get: { filter.noCollection }, set: { on in change { $0.setNoCollection(on) } }))
                .toggleStyle(.checkbox)
            }
            if model.shownSmart != nil {
                row("Show") {
                    ForEach([ShouciCore.LibraryView.active, .archived, .all], id: \.self) { view in
                        ToggleChip(title: Self.viewTitle(view), isOn: filter.view == view) {
                            change { $0.view = view }
                        }
                    }
                }
            }
        }
    }

    private var footer: some View {
        HStack {
            Button("Clear All", action: model.clearFilter)
                .disabled(!filter.hasConditions)
            Spacer()
            if model.shownSmart != nil {
                Button("Save Changes", action: model.saveSmartChanges)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.hasUnsavedSmartChanges)
            } else if model.scope != .trash {
                Button("Save as Smart Collection…", action: model.saveAsSmartCollection)
                    .disabled(!filter.hasConditions)
            }
        }
        .controlSize(.small)
    }

    private func change(_ edit: (inout LibraryFilter) -> Void) {
        var filter = model.filter
        edit(&filter)
        model.setFilter(filter)
    }

    private func toggled<T: Equatable>(_ values: [T], _ value: T) -> [T] {
        values.contains(value) ? values.filter { $0 != value } : values + [value]
    }

    private func row(_ title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            CardLabel(title)
            FlowLayout(spacing: 5) { content() }
        }
    }

    private func section(_ title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            CardLabel(title)
            content()
        }
    }

    private static func periodTitle(_ days: UInt32) -> String {
        switch days {
        case 7: "Past Week"
        case 30: "Past Month"
        case 90: "3 Months"
        case 365: "Past Year"
        default: "\(days) Days"
        }
    }

    private static func viewTitle(_ view: ShouciCore.LibraryView) -> String {
        switch view {
        case .active: "Active"
        case .archived: "Archived"
        case .all, .trash: "Both"
        }
    }
}

/// A value the filter can ask for, on or off.
private struct ToggleChip: View {
    let title: String
    let isOn: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(isOn ? Typography.controlBold : Typography.control)
                .foregroundStyle(isOn ? Palette.onAccent : Palette.text)
                .padding(.horizontal, 9)
                .frame(height: 22)
                .background(Capsule().fill(isOn ? Palette.accentFill : Palette.field))
                .overlay { if !isOn { Capsule().strokeBorder(Palette.line) } }
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isOn ? .isSelected : [])
    }
}

/// Tags or collections the filter names, each with a button that takes it
/// off, then a menu of the others.
private struct NameRow: View {
    let title: String
    let kind: GroupKind
    let names: [String]
    let known: [GroupView]
    let add: (String) -> Void
    let remove: (String) -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(title)
                .foregroundStyle(Palette.secondary)
                .frame(width: 46, alignment: .leading)
            FlowLayout(spacing: 5) {
                ForEach(names, id: \.self) { name in
                    Chip(text: name, style: kind.chipStyle) { remove(name) }
                }
                let others = known.filter { group in !names.contains(group.name) }
                if !others.isEmpty {
                    Menu {
                        ForEach(others) { group in
                            Button(group.name) { add(group.name) }
                        }
                    } label: {
                        Label(kind == .tag ? "Tag" : "Collection", systemImage: "plus")
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel(kind == .tag ? "Add a tag to “\(title)”" : "Add a collection to “\(title)”")
                } else if names.isEmpty {
                    Text(kind == .tag ? "No tags yet" : "No collections yet")
                        .foregroundStyle(Palette.tertiary)
                        .frame(height: 22)
                }
            }
        }
    }
}

/// The filter above the list: its conditions as chips that take them off,
/// how many words they leave, and saving them as a smart collection (or
/// saving changes to the one shown).
struct FilterBar: View {
    @Bindable var model: LibraryModel

    var body: some View {
        let conditions = model.filter.conditions
        VStack(alignment: .leading, spacing: 7) {
            FlowLayout(spacing: 6) {
                if conditions.isEmpty {
                    Chip(text: "Every word", style: .plain)
                }
                ForEach(conditions) { condition in
                    Chip(text: condition.label) { model.setFilter(condition.without) }
                }
            }
            if let smart = model.shownSmart {
                if model.hasUnsavedSmartChanges {
                    HStack(spacing: 8) {
                        Text("Unsaved changes")
                            .foregroundStyle(Palette.tertiary)
                            .lineLimit(1)
                            .help("Changes to “\(smart.name)” not saved yet")
                        Spacer()
                        Button("Revert", action: model.revertSmartChanges)
                        Button("Save Changes", action: model.saveSmartChanges)
                    }
                }
            } else {
                HStack(spacing: 8) {
                    Text("\(model.shownCount) of \(model.count(of: model.scope))")
                        .foregroundStyle(Palette.tertiary)
                        .monospacedDigit()
                    Spacer()
                    Button("Clear", action: model.clearFilter)
                    if model.scope != .trash {
                        Button("Save as Smart Collection…", action: model.saveAsSmartCollection)
                    }
                }
            }
        }
        .controlSize(.small)
        .font(Typography.control)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.bar)
        .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 1) }
    }
}

// Conditions that can't both hold take each other off: a tag or collection
// asked for and kept out, or a collection and none (which `shouci` refuses).
extension LibraryFilter {
    /// Asks for words with this tag too, any of them doing.
    mutating func addAnyTag(_ name: String) {
        anyTags.append(name)
        withoutTags.removeAll { sameName($0, name) }
    }

    /// Keeps out words with this tag.
    mutating func addWithoutTag(_ name: String) {
        withoutTags.append(name)
        anyTags.removeAll { sameName($0, name) }
        tags.removeAll { sameName($0, name) }
    }

    /// Asks for words in this collection too, any of them doing.
    mutating func addAnyCollection(_ name: String) {
        anyCollections.append(name)
        withoutCollections.removeAll { sameName($0, name) }
        noCollection = false
    }

    /// Keeps out words in this collection.
    mutating func addWithoutCollection(_ name: String) {
        withoutCollections.append(name)
        anyCollections.removeAll { sameName($0, name) }
        if let collection, sameName(collection, name) { self.collection = nil }
    }

    /// Asks for words in no collection, or stops asking.
    mutating func setNoCollection(_ on: Bool) {
        noCollection = on
        if on {
            anyCollections = []
            collection = nil
        }
    }
}

/// Names as the library compares them, ignoring case.
private func sameName(_ a: String, _ b: String) -> Bool {
    a.caseInsensitiveCompare(b) == .orderedSame
}
