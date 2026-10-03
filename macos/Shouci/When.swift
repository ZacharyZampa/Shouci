import Foundation

/// Times the way a list shows them: `Just now`, `9:38 AM`, `Yesterday`,
/// `Sep 21`.
enum When {
    static func short(_ timestamp: String, now: Date = .now) -> String {
        guard let date = parse(timestamp) else { return "" }
        if now.timeIntervalSince(date) < 60 { return "Just now" }
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return date.formatted(date: .omitted, time: .shortened) }
        if calendar.isDateInYesterday(date) { return "Yesterday" }
        if calendar.isDate(date, equalTo: now, toGranularity: .year) {
            return date.formatted(.dateTime.month(.abbreviated).day())
        }
        return date.formatted(date: .abbreviated, time: .omitted)
    }

    /// The core's timestamps: `2026-10-01T09:38:12.345Z`.
    static func parse(_ timestamp: String) -> Date? {
        try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(timestamp)
    }
}
