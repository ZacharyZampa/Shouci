import AppKit
import ShouciCore
import SwiftUI

/// A tag, a collection, or a status, as a small capsule.
struct Chip: View {
    enum Style { case accent, plain, ok, warn }

    let text: String
    var icon: String?
    var style: Style = .accent
    var remove: (() -> Void)?

    var body: some View {
        HStack(spacing: 4) {
            if let icon { Image(systemName: icon).font(.system(size: 10, weight: .bold)) }
            Text(text).lineLimit(1)
            if let remove {
                Button(action: remove) {
                    // A small mark with the chip's full height to click: the
                    // Mac's 20-point minimum.
                    Image(systemName: "xmark").font(.system(size: 8, weight: .bold))
                        .frame(width: 20, height: 22)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(.leading, -4)
                .accessibilityLabel("Remove \(text)")
            }
        }
        .font(style == .ok || style == .warn ? Typography.controlBold : Typography.control)
        .foregroundStyle(foreground)
        .padding(.leading, icon == nil ? 9 : 8)
        .padding(.trailing, remove == nil ? 9 : 1)
        .frame(height: 22)
        .background(Capsule().fill(background))
        .overlay { if style == .plain { Capsule().strokeBorder(Palette.line) } }
    }

    private var foreground: Color {
        switch style {
        case .accent: Palette.accent
        case .plain: Palette.text
        case .ok: Palette.ok
        case .warn: Palette.warn
        }
    }

    private var background: Color {
        switch style {
        case .accent: Palette.accentSoft
        case .plain: Palette.field
        case .ok: Palette.okSoft
        case .warn: Palette.warnSoft
        }
    }
}

/// Lays children out in rows, wrapping when a row is full. A child wider than
/// a whole row is offered the row's width, so a long name truncates rather
/// than running past the edge.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(subviews, width: proposal.width ?? .infinity)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(subviews, width: bounds.width) {
            var x = bounds.minX
            for index in row.indices {
                let size = Self.size(of: subviews[index], within: bounds.width)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var indices: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> [Row] {
        var rows: [Row] = [Row()]
        for index in subviews.indices {
            let size = Self.size(of: subviews[index], within: width)
            let needed = rows[rows.count - 1].indices.isEmpty ? size.width : rows[rows.count - 1].width + spacing + size.width
            if needed > width && !rows[rows.count - 1].indices.isEmpty {
                rows.append(Row())
            }
            var row = rows[rows.count - 1]
            row.width = row.indices.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.indices.append(index)
            rows[rows.count - 1] = row
        }
        return rows.filter { !$0.indices.isEmpty }
    }

    private static func size(of subview: LayoutSubview, within width: CGFloat) -> CGSize {
        let ideal = subview.sizeThatFits(.unspecified)
        guard ideal.width > width else { return ideal }
        return subview.sizeThatFits(ProposedViewSize(width: width, height: nil))
    }
}

/// How a query is read: shown as a chip, changed from its menu (mockup 05).
struct KindMenu: View {
    @Binding var kind: QueryKind?
    let shown: QueryKind

    var body: some View {
        Menu {
            Picker("Search as", selection: $kind) {
                Text("Automatic").tag(QueryKind?.none)
                ForEach([QueryKind.chinese, .pinyin, .english], id: \.self) { kind in
                    Text(kind.title).tag(QueryKind?.some(kind))
                }
            }
            .pickerStyle(.inline)
        } label: {
            HStack(spacing: 4) {
                Text(shown.title)
                Image(systemName: "chevron.down").font(.system(size: 8, weight: .bold))
            }
            .font(Typography.controlBold)
            .foregroundStyle(Palette.accent)
            .padding(.horizontal, 9)
            .frame(height: 22)
            .background(Capsule().fill(Palette.accentSoft))
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel("Search as \(shown.title)")
    }
}

/// A small heading over a card: `Your entry`, `Dictionary`.
struct CardLabel: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(Typography.label)
            .foregroundStyle(Palette.tertiary)
            .accessibilityAddTraits(.isHeader)
    }
}

extension View {
    /// The mockups' card: list background, hairline border, 12pt corners.
    /// The border is drawn behind the content, so a menu opening from a
    /// field inside covers it.
    func card() -> some View {
        background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Palette.list)
                .strokeBorder(Palette.line))
    }
}

/// What the library window leaves its sheets and the filter popover: the
/// width of its screen, and the height from the foot of its toolbar, where a
/// sheet hangs, to the foot of the screen. Measured from the window as it
/// is; a small display, or one set to Larger Text (1024 × 640 points on a
/// 13-inch Mac), has less than the mockups.
@MainActor
struct ScreenRoom {
    /// Kept clear between a sheet or popover and the screen's edges.
    static let margin: CGFloat = 20
    /// Never less than this, even below a window parked at the foot of the
    /// screen: there, what's inside scrolls.
    static let least: CGFloat = 320
    /// The library window, set by its controller.
    static weak var window: NSWindow?

    let width: CGFloat
    let belowToolbar: CGFloat

    static var current: ScreenRoom {
        guard let window, let screen = window.screen ?? NSScreen.main else {
            let visible = NSScreen.main?.visibleFrame.size ?? CGSize(width: 1440, height: 900)
            return ScreenRoom(width: visible.width, belowToolbar: visible.height)
        }
        let visible = screen.visibleFrame
        // The content below the title bar and toolbar, in screen coordinates.
        let toolbarFoot = window.frame.minY + window.contentLayoutRect.maxY
        return ScreenRoom(width: visible.width, belowToolbar: max(toolbarFoot - visible.minY, least))
    }
}

extension View {
    /// A sheet `width` by `height` where the screen has room for it, and only
    /// as big as the screen allows where it doesn't, so the buttons along its
    /// foot stay on screen. What's inside must scroll.
    func sheetFrame(width: CGFloat, height: CGFloat) -> some View {
        let room = ScreenRoom.current
        return frame(
            width: min(width, room.width - 2 * ScreenRoom.margin),
            height: min(height, room.belowToolbar - ScreenRoom.margin))
    }

    /// The content as it is while it fits in `room` points of height, and
    /// scrolling in that much once it is taller: a popover with many tags,
    /// or on a short screen.
    func scrollsBeyond(_ room: CGFloat) -> some View {
        modifier(ScrollsBeyond(room: room))
    }
}

private struct ScrollsBeyond: ViewModifier {
    let room: CGFloat
    @State private var height: CGFloat = 0

    func body(content: Content) -> some View {
        let measured = content.onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
        if height > room {
            ScrollView { measured }.frame(height: room)
        } else {
            measured
        }
    }
}

/// A warning in a sheet: what went wrong, where it is seen.
struct Notice: View {
    var icon = "exclamationmark.triangle"
    let text: String

    var body: some View {
        Label(text, systemImage: icon)
            .font(Typography.content)
            .foregroundStyle(Palette.warn)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.warnSoft))
    }
}
