import AppKit
import SwiftUI

/// The mockups' palette, light and dark (each color's `Swatch`). Its indigo
/// is also the app's accent color (`AccentColor` in the asset catalog), so
/// buttons, switches, selection, and focus rings are the same indigo.
enum Palette {
    static let list = Swatch.list.color
    static let window = Swatch.window.color
    static let field = Swatch.field.color
    static let detail = Swatch.detail.color
    static let line = Swatch.line.color
    static let text = Swatch.text.color
    static let secondary = Swatch.secondary.color
    static let tertiary = Swatch.tertiary.color
    static let accent = Swatch.accent.color
    static let accentFill = Swatch.accentFill.color
    static let accentSoft = Swatch.accentSoft.color
    static let ok = Swatch.ok.color
    static let okSoft = Swatch.okSoft.color
    static let warn = Swatch.warn.color
    static let warnSoft = Swatch.warnSoft.color
    /// Text on `accentFill`.
    static let onAccent = Color.white
    static let onAccentSecondary = Swatch.onAccentSecondary.color
    /// A key or a button drawn on the selection: darker than it, so its
    /// white text reads.
    static let onAccentInset = Swatch.onAccentInset.color
}

/// A palette color in each appearance. With Increase Contrast on, the colors
/// that carry text or edges take their `contrast` pair: text reads at 7:1 or
/// more on every surface it sits on, and hairlines at 3:1.
struct Swatch: Sendable {
    typealias Shade = (hex: Int, alpha: Double)

    let light: Shade
    let dark: Shade
    let contrast: (light: Shade, dark: Shade)?

    init(light: Int, dark: Int, contrast: (light: Int, dark: Int)? = nil) {
        self.init(light: (light, 1), dark: (dark, 1), contrast: contrast.map { (($0.light, 1), ($0.dark, 1)) })
    }

    init(light: Shade, dark: Shade, contrast: (light: Shade, dark: Shade)? = nil) {
        self.light = light
        self.dark = dark
        self.contrast = contrast
    }

    static let list = Swatch(light: 0xFFFFFF, dark: 0x1F1F23)
    static let window = Swatch(light: 0xF6F6F9, dark: 0x2A2A30)
    static let field = Swatch(light: 0xFFFFFF, dark: 0x34343A)
    static let detail = Swatch(light: 0xFAFAFC, dark: 0x252529)
    static let line = Swatch(
        light: (0x000000, 0.10), dark: (0xFFFFFF, 0.11), contrast: ((0x000000, 0.45), (0xFFFFFF, 0.50)))
    static let text = Swatch(light: 0x1D1D20, dark: 0xF4F4F7, contrast: (0x000000, 0xFFFFFF))
    static let secondary = Swatch(light: 0x4F4F58, dark: 0xC4C4CC, contrast: (0x2E2E35, 0xE2E2E8))
    static let tertiary = Swatch(light: 0x63636D, dark: 0xA3A3AD, contrast: (0x3A3A42, 0xD2D2DA))
    static let accent = Swatch(light: 0x3446C8, dark: 0x98A5FF, contrast: (0x1F2C9E, 0xD0D6FF))
    static let accentFill = Swatch(light: 0x3B4FD8, dark: 0x4357E6, contrast: (0x2A3BB8, 0x2A3BB8))
    static let accentSoft = Swatch(
        light: (0x3B4FD8, 0.10), dark: (0x98A5FF, 0.16), contrast: ((0x3B4FD8, 0.16), (0x98A5FF, 0.16)))
    static let ok = Swatch(light: 0x17703F, dark: 0x6FD9A0, contrast: (0x0B4E2A, 0xAEF5D0))
    static let okSoft = Swatch(
        light: (0x17703F, 0.11), dark: (0x6FD9A0, 0.15), contrast: ((0x17703F, 0.16), (0x6FD9A0, 0.15)))
    static let warn = Swatch(light: 0x9A4F00, dark: 0xFFB454, contrast: (0x6B3600, 0xFFDCA6))
    static let warnSoft = Swatch(
        light: (0xD27800, 0.14), dark: (0xFFB454, 0.16), contrast: ((0xD27800, 0.20), (0xFFB454, 0.16)))
    static let onAccentSecondary = Swatch(
        light: (0xFFFFFF, 0.84), dark: (0xFFFFFF, 0.90), contrast: ((0xFFFFFF, 1), (0xFFFFFF, 1)))
    static let onAccentInset = Swatch(light: (0x000000, 0.18), dark: (0x000000, 0.18))

    func shade(dark isDark: Bool, increasedContrast: Bool) -> Shade {
        let plain: (light: Shade, dark: Shade) = (light, dark)
        let pair = increasedContrast ? contrast ?? plain : plain
        return isDark ? pair.dark : pair.light
    }

    /// Follows the appearance: Increase Contrast shows as its high-contrast
    /// variants, which an appearance can only be matched against.
    var color: Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let match = appearance.bestMatch(from: [
                .aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua,
            ])
            let (hex, alpha) = shade(
                dark: match == .darkAqua || match == .accessibilityHighContrastDarkAqua,
                increasedContrast: match == .accessibilityHighContrastAqua
                    || match == .accessibilityHighContrastDarkAqua)
            return NSColor(
                srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
                green: CGFloat((hex >> 8) & 0xFF) / 255,
                blue: CGFloat(hex & 0xFF) / 255,
                alpha: alpha)
        })
    }
}

/// The type roles, from the word itself down to labels: one size and weight
/// each, wherever the role appears. Sizes come from the mockups; glyphs in
/// chips and banners are sized with their text, not here.
enum Typography {
    /// The word, in the detail pane.
    static let hero = Font.system(size: 60, weight: .medium)
    /// A word heading a card: the filing sheet, the editor's dictionary.
    static let display = Font.system(size: 28, weight: .medium)
    /// A count in the import summary.
    static let figure = Font.system(size: 26, weight: .semibold)
    /// The reading beside the word in the detail pane.
    static let heroReading = Font.system(size: 24)
    /// Characters being typed in the editor.
    static let field = Font.system(size: 22)
    /// Sheet and pane titles.
    static let title = Font.system(size: 20, weight: .semibold)
    /// A word in a list row.
    static let headword = Font.system(size: 18, weight: .semibold)
    /// A reading or traditional form beside a larger word.
    static let alternate = Font.system(size: 17)
    /// A banner's or result's headline, an option's name.
    static let emphasis = Font.system(size: 14, weight: .semibold)
    /// What a card or sheet says: definitions, senses, notes, actions.
    static let content = Font.system(size: 13.5)
    /// The reading beside a word in a list row.
    static let reading = Font.system(size: 13)
    /// Under a word or a heading: its meaning, a description.
    static let supporting = Font.system(size: 12.5)
    /// Chips, buttons, hints, footers, and form labels.
    static let control = Font.system(size: 12)
    static let controlBold = Font.system(size: 12, weight: .semibold)
    /// Labels over cards, lists, and columns.
    static let label = Font.system(size: 11, weight: .semibold)
    /// Key caps and the smallest metadata.
    static let key = Font.system(size: 11)
}

/// Which characters a piece of Chinese is written in.
enum ChineseScript {
    case simplified
    case traditional
}

extension View {
    /// Chinese set as Chinese, in the glyph forms of `script`. Untagged, Han
    /// characters take their forms from the Mac's language order, which can
    /// give Japanese ones (骨, 直, 角), or mainland ones for traditional text.
    func chinese(_ script: ChineseScript = .simplified) -> some View {
        typesettingLanguage(Locale.Language(identifier: script == .simplified ? "zh-Hans" : "zh-Hant"))
    }
}

/// A key as printed on a keyboard: `⌘O`, `esc`, `↵`.
struct KeyCap: View {
    let label: String
    var size: Size = .small

    enum Size { case small, large }

    var body: some View {
        Text(label)
            .font(size == .small ? Typography.key : .body)
            .foregroundStyle(size == .small ? Palette.secondary : Palette.text)
            .padding(.horizontal, size == .small ? 5 : 0)
            .frame(minWidth: size == .small ? 18 : 28, minHeight: size == .small ? 18 : 28)
            .background(
                RoundedRectangle(cornerRadius: size == .small ? 5 : 7, style: .continuous)
                    .fill(Palette.field)
                    .strokeBorder(Palette.line))
    }
}

/// A key and what it does, for the panel's footer.
struct KeyHint: View {
    let key: String
    let action: String

    var body: some View {
        HStack(spacing: 6) {
            KeyCap(label: key)
            Text(action)
        }
        .accessibilityElement(children: .combine)
    }
}
