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
                    Image(systemName: "xmark").font(.system(size: 8, weight: .bold))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Remove \(text)")
            }
        }
        .font(.system(size: 12, weight: style == .ok || style == .warn ? .semibold : .regular))
        .foregroundStyle(foreground)
        .padding(.leading, icon == nil ? 9 : 8)
        .padding(.trailing, remove == nil ? 9 : 7)
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

/// Lays children out in rows, wrapping when a row is full.
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
                let size = subviews[index].sizeThatFits(.unspecified)
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
            let size = subviews[index].sizeThatFits(.unspecified)
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
            .font(.system(size: 12, weight: .semibold))
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
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(Palette.tertiary)
            .accessibilityAddTraits(.isHeader)
    }
}

extension View {
    /// The mockups' card: list background, hairline border, 12pt corners.
    func card() -> some View {
        background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Palette.list))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line))
    }
}
