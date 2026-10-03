import AppKit
import SwiftUI

/// The mockups' palette, light and dark.
enum Palette {
    static let list = dynamic(light: 0xFFFFFF, dark: 0x1F1F23)
    static let window = dynamic(light: 0xF6F6F9, dark: 0x2A2A30)
    static let field = dynamic(light: 0xFFFFFF, dark: 0x34343A)
    static let detail = dynamic(light: 0xFAFAFC, dark: 0x252529)
    static let line = dynamic(light: (0x000000, 0.10), dark: (0xFFFFFF, 0.11))
    static let text = dynamic(light: 0x1D1D20, dark: 0xF4F4F7)
    static let secondary = dynamic(light: 0x4F4F58, dark: 0xC4C4CC)
    static let tertiary = dynamic(light: 0x63636D, dark: 0xA3A3AD)
    static let accent = dynamic(light: 0x3446C8, dark: 0x98A5FF)
    static let accentFill = dynamic(light: 0x3B4FD8, dark: 0x4357E6)
    static let accentSoft = dynamic(light: (0x3B4FD8, 0.10), dark: (0x98A5FF, 0.16))
    static let ok = dynamic(light: 0x17703F, dark: 0x6FD9A0)
    static let okSoft = dynamic(light: (0x17703F, 0.11), dark: (0x6FD9A0, 0.15))
    static let warn = dynamic(light: 0x9A4F00, dark: 0xFFB454)
    static let warnSoft = dynamic(light: (0xD27800, 0.14), dark: (0xFFB454, 0.16))
    /// Text on `accentFill`.
    static let onAccent = Color.white
    static let onAccentSecondary = Color.white.opacity(0.84)

    private static func dynamic(light: Int, dark: Int) -> Color {
        dynamic(light: (light, 1), dark: (dark, 1))
    }

    private static func dynamic(light: (Int, Double), dark: (Int, Double)) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            let (hex, alpha) = isDark ? dark : light
            return NSColor(
                srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
                green: CGFloat((hex >> 8) & 0xFF) / 255,
                blue: CGFloat(hex & 0xFF) / 255,
                alpha: alpha)
        })
    }
}

/// A key as printed on a keyboard: `⌘O`, `esc`, `↵`.
struct KeyCap: View {
    let label: String
    var size: Size = .small

    enum Size { case small, large }

    var body: some View {
        Text(label)
            .font(.system(size: size == .small ? 11 : 13))
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
