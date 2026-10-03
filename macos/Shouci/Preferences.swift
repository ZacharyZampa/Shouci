import Foundation
import Observation

/// What the panel does after a word is saved.
enum AfterSave: String, CaseIterable, Identifiable {
    case stayOpen
    case close

    var id: Self { self }

    var title: String {
        switch self {
        case .stayOpen: "Stay open for the next word"
        case .close: "Close the panel"
        }
    }
}

/// How pinyin is written.
enum PinyinStyle: String, CaseIterable, Identifiable {
    case marks
    case numbers

    var id: Self { self }

    var title: String {
        switch self {
        case .marks: "Tone marks (xí guàn)"
        case .numbers: "Tone numbers (xi2 guan4)"
        }
    }
}

/// This app's own settings, kept in `UserDefaults`. Settings every frontend
/// shares (which dictionaries are on) live in the core.
@MainActor
@Observable
final class Preferences {
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored var showMenuBarIconChanged: (Bool) -> Void = { _ in }

    var showMenuBarIcon: Bool {
        didSet {
            defaults.set(showMenuBarIcon, forKey: Keys.showMenuBarIcon)
            showMenuBarIconChanged(showMenuBarIcon)
        }
    }

    var afterSave: AfterSave {
        didSet { defaults.set(afterSave.rawValue, forKey: Keys.afterSave) }
    }

    var pinyinStyle: PinyinStyle {
        didSet { defaults.set(pinyinStyle.rawValue, forKey: Keys.pinyinStyle) }
    }

    /// Show the traditional form beside the simplified one in lists.
    var showTraditional: Bool {
        didSet { defaults.set(showTraditional, forKey: Keys.showTraditional) }
    }

    private(set) var hotKey: HotKeyBinding

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        showMenuBarIcon = defaults.object(forKey: Keys.showMenuBarIcon) as? Bool ?? true
        afterSave = defaults.string(forKey: Keys.afterSave).flatMap(AfterSave.init) ?? .stayOpen
        pinyinStyle = defaults.string(forKey: Keys.pinyinStyle).flatMap(PinyinStyle.init) ?? .marks
        showTraditional = defaults.bool(forKey: Keys.showTraditional)
        hotKey = HotKeyBinding.load(from: defaults)
    }

    /// A reading in the chosen style: the core gives both.
    func pinyin(raw: String, display: String) -> String {
        pinyinStyle == .numbers ? raw : display
    }

    /// Characters for a list row: simplified, then traditional when asked
    /// for and different.
    func headword(simplified: String, traditional: String) -> String {
        guard showTraditional, !traditional.isEmpty, traditional != simplified else { return simplified }
        return "\(simplified) \(traditional)"
    }

    func setHotKey(_ binding: HotKeyBinding) {
        hotKey = binding
        binding.store(in: defaults)
    }

    private enum Keys {
        static let showMenuBarIcon = "ShouciShowMenuBarIcon"
        static let afterSave = "ShouciAfterSave"
        static let pinyinStyle = "ShouciPinyinStyle"
        static let showTraditional = "ShouciShowTraditional"
    }
}
