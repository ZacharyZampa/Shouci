import AppKit
import ShouciCore
import SwiftUI
import Testing

/// What VoiceOver hears, Increase Contrast, and long names.
@MainActor
struct AccessibilityTests {
    // MARK: Increase Contrast

    private static let surfaces: [Swatch] = [.list, .window, .field, .detail]

    @Test(arguments: [false, true])
    func underIncreaseContrastTextReadsAtSevenToOneAndEdgesAtThree(_ dark: Bool) {
        func shade(_ swatch: Swatch, over ground: RGB? = nil) -> RGB {
            rgb(swatch.shade(dark: dark, increasedContrast: true), over: ground)
        }
        for surface in Self.surfaces {
            let ground = shade(surface)
            for text: Swatch in [.text, .secondary, .tertiary, .accent, .ok, .warn] {
                #expect(contrast(shade(text, over: ground), ground) >= 7)
            }
            #expect(contrast(shade(.line, over: ground), ground) >= 3)
        }
        // Tinted chips and banners sit on the list, window, and detail, never
        // inside a field.
        for surface: Swatch in [.list, .window, .detail] {
            let ground = shade(surface)
            for (text, fill) in [(Swatch.accent, Swatch.accentSoft), (.ok, .okSoft), (.warn, .warnSoft)] {
                let tinted = shade(fill, over: ground)
                #expect(contrast(shade(text, over: tinted), tinted) >= 7)
            }
        }
        let selected = shade(.accentFill)
        #expect(contrast(shade(.onAccentSecondary, over: selected), selected) >= 7)
    }

    @Test(arguments: [false, true])
    func whiteTextOnTheSelectionReadsAtFourAndAHalf(_ dark: Bool) {
        func shade(_ swatch: Swatch, over ground: RGB? = nil) -> RGB {
            rgb(swatch.shade(dark: dark, increasedContrast: false), over: ground)
        }
        let selected = shade(.accentFill)
        let white = RGB(red: 1, green: 1, blue: 1)
        #expect(contrast(white, selected) >= 4.5)
        #expect(contrast(shade(.onAccentSecondary, over: selected), selected) >= 4.5)
        // A key cap or an Add button on the selection.
        #expect(contrast(white, shade(.onAccentInset, over: selected)) >= 4.5)
    }

    /// Buttons, switches, and selection are drawn in the asset catalog's
    /// accent color: the palette's indigo.
    @Test func theAppsAccentColorIsThePalettesIndigo() throws {
        let accent = try #require(NSColor(named: "AccentColor", bundle: Bundle(for: TestLibrary.self)))
        for (dark, name) in [(false, NSAppearance.Name.aqua), (true, .darkAqua)] {
            let appearance = try #require(NSAppearance(named: name))
            var hex = 0
            appearance.performAsCurrentDrawingAppearance {
                let srgb = accent.usingColorSpace(.sRGB) ?? .clear
                let channel = { (value: CGFloat) in Int((value * 255).rounded()) }
                hex = channel(srgb.redComponent) << 16 | channel(srgb.greenComponent) << 8 | channel(srgb.blueComponent)
            }
            #expect(hex == Swatch.accentFill.shade(dark: dark, increasedContrast: false).hex)
        }
    }

    @Test func theAppearanceChoosesTheShade() throws {
        let aqua = try #require(NSAppearance(named: .aqua))
        let darkAqua = try #require(NSAppearance(named: .darkAqua))
        #expect(drawn(Palette.text, in: aqua) == Swatch.text.shade(dark: false, increasedContrast: false).hex)
        #expect(drawn(Palette.text, in: darkAqua) == Swatch.text.shade(dark: true, increasedContrast: false).hex)
    }

    // MARK: What VoiceOver hears

    @Test func quickSearchSaysTheWordItsReadingMeaningAndWhetherItIsSaved() throws {
        let library = try TestLibrary()
        let search = QuickSearch(app: library.model.app, preferences: library.model.preferences)
        #expect(search.spoken(.candidate(candidate())) == "习惯, xíguàn, habit")
        let saved = SavedRef(id: 1, verification: .confirmed, lifecycle: .active)
        #expect(search.spoken(.candidate(candidate(saved: saved))) == "习惯, xíguàn, habit, saved")
        let trashed = SavedRef(id: 1, verification: .confirmed, lifecycle: .trashed)
        #expect(search.spoken(.candidate(candidate(saved: trashed))) == "习惯, xíguàn, habit, in the Trash")
        #expect(search.spoken(.saveForReview("饕餮")) == "Save “饕餮” as needs review")
    }

    @Test func aSaveIsSaidAsTheBannerSaysIt() throws {
        let library = try TestLibrary()
        let core = library.core
        let before = try core.snapshot(ids: [])
        let result = try core.addManual(word: ManualWord(simplified: "饕餮", pinyin: "tao1 tie4"))
        let saved = QuickSearch.Saved(
            result: result, before: before, after: try core.snapshot(ids: [result.item.id]))
        #expect(saved.spoken(library.model.preferences).hasPrefix("Saved 饕餮, "))
    }

    @Test func aTypedNameSaysWhatItFoundAndWhatReturnPicks() {
        let known = [GroupView(name: "HSK 1", count: 12), GroupView(name: "HSK 2", count: 1)]
        #expect(NameSuggestions(query: "hsk", known: known, present: []).spoken(.tag) == "2 tags. HSK 1, 12 words")
        #expect(NameSuggestions(query: "hsk 2", known: known, present: []).spoken(.tag) == "1 tag. HSK 2, 1 word")
        #expect(
            NameSuggestions(query: "Travel", known: known, present: []).spoken(.collection)
                == "No collection matches. Create collection “Travel”")
    }

    @Test func filingSaysWhoseTurnItIs() async throws {
        let library = try TestLibrary()
        try library.add("书")
        try library.add("笔")
        await library.model.reload()
        library.model.scope = .noCollection
        library.model.startFiling()
        #expect(library.model.spokenFilingTurn.hasPrefix("Word 1 of 2: "))
        library.model.skipFiling()
        library.model.skipFiling()
        #expect(library.model.spokenFilingTurn == "Every word had its turn")
    }

    // MARK: Small screens

    @Test func quickSearchAsksForNoMoreResultsThanFitBelowThePanel() throws {
        let library = try TestLibrary()
        let search = QuickSearch(app: library.model.app, preferences: library.model.preferences)
        search.fit(height: 900)
        #expect(search.limit == 8)
        // A 13-inch Mac at Larger Text, the Dock along the bottom.
        search.fit(height: 560)
        #expect(search.limit == 5)
        search.fit(height: 200)
        #expect(search.limit == 3)
    }

    /// The one part of the panel `fit` can't take from a fixed frame: a
    /// banner with a line of detail and an Undo button.
    @Test func aBannerTakesNoMoreThanFitAllowsForIt() {
        let banner = Banner(
            icon: "checkmark.circle", tint: Palette.ok, fill: Palette.okSoft, title: "Saved 习惯 · xíguàn",
            detail: "habit; custom; usual practice"
        ) {
            Button("Undo") {}.buttonStyle(PillButtonStyle())
        }
        let size = NSHostingController(rootView: banner.frame(width: 440)).sizeThatFits(
            in: CGSize(width: 440, height: 1000))
        #expect(size.height <= QuickSearchView.Metrics.banner)
    }

    @Test func recentlyAddedWordsAreKeptToWhatFits() async throws {
        let library = try TestLibrary()
        for word in ["一", "二", "三", "四", "五", "六"] { try library.add(word) }
        let search = QuickSearch(app: library.model.app, preferences: library.model.preferences)
        search.opened()
        for _ in 0..<200 where search.recent.count < 5 { try await Task.sleep(for: .milliseconds(10)) }
        #expect(search.recentShown.count == 5)
        search.fit(height: 200)
        #expect(search.recentShown.count == 3)
    }

    // MARK: Long names

    @Test func aNameLongerThanTheRowTruncatesInsideIt() {
        let name = String(repeating: "Integrated Chinese ", count: 8)
        let chips = NSHostingController(rootView: FlowLayout { Chip(text: name); Chip(text: "HSK 1") })
        let size = chips.sizeThatFits(in: CGSize(width: 180, height: 1000))
        #expect(size.width <= 180)
    }

    // MARK: Helpers

    private func candidate(saved: SavedRef? = nil) -> CandidateView {
        CandidateView(
            dictionary: "cc-cedict", dictionaryVersion: "1", simplified: "习惯", traditional: "習慣",
            pinyin: "xi2 guan4", pinyinDisplay: "xíguàn", glosses: ["habit"], definitionDisplay: "habit",
            frequencyRank: nil, hskRank: nil, inferred: false, basis: .simplified, saved: saved)
    }

    private struct RGB {
        let red: Double
        let green: Double
        let blue: Double
    }

    /// A shade over `ground` (white when none is given).
    private func rgb(_ shade: Swatch.Shade, over ground: RGB? = nil) -> RGB {
        let under = ground ?? RGB(red: 1, green: 1, blue: 1)
        func mix(_ channel: Int, _ below: Double) -> Double {
            Double(channel & 0xFF) / 255 * shade.alpha + below * (1 - shade.alpha)
        }
        return RGB(
            red: mix(shade.hex >> 16, under.red), green: mix(shade.hex >> 8, under.green),
            blue: mix(shade.hex, under.blue))
    }

    /// `color` as drawn in `appearance`, as a hex value.
    private func drawn(_ color: Color, in appearance: NSAppearance) -> Int {
        var resolved = NSColor.clear
        appearance.performAsCurrentDrawingAppearance {
            resolved = NSColor(color).usingColorSpace(.sRGB) ?? .clear
        }
        let channel = { (value: CGFloat) in Int((value * 255).rounded()) }
        return channel(resolved.redComponent) << 16 | channel(resolved.greenComponent) << 8
            | channel(resolved.blueComponent)
    }

    /// WCAG's contrast ratio.
    private func contrast(_ a: RGB, _ b: RGB) -> Double {
        func luminance(_ c: RGB) -> Double {
            func channel(_ v: Double) -> Double { v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
            return 0.2126 * channel(c.red) + 0.7152 * channel(c.green) + 0.0722 * channel(c.blue)
        }
        let (x, y) = (luminance(a), luminance(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}
