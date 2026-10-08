// Swift conveniences over the generated bindings (ShouciCore.swift).

extension ShouciError {
    public var kind: ErrorKind {
        switch self {
        case .Failed(let kind, _): kind
        }
    }

    /// Written for people.
    public var message: String {
        switch self {
        case .Failed(_, let message): message
        }
    }
}

/// What to tell people about a failed call.
public func describe(_ error: any Error) -> String {
    (error as? ShouciError)?.message ?? error.localizedDescription
}

// The generated initializers default every optional field. Enum fields
// can't be given defaults from Rust, so these fill in the usual ones.

extension LibraryFilter {
    /// Active words: neither archived nor in the trash.
    public init() {
        self.init(view: .active)
    }
}

extension ExportRequest {
    /// Active words, confirmed only.
    public init(scope: ExportScope = .new) {
        self.init(scope: scope, filter: LibraryFilter())
    }
}

extension ImportField {
    /// The field's name for people: `definition`, `notes`.
    public var name: String {
        switch self {
        case .definition: "definition"
        case .notes: "notes"
        }
    }
}

extension ItemView: Identifiable {}
extension DictionaryView: Identifiable {}
extension ConnectorView: Identifiable {}

extension GroupView: Identifiable {
    public var id: String { name }
}

extension SmartCollectionView: Identifiable {
    public var id: String { name }
}

extension FilterConditionView: Identifiable {}

extension FrequencyBandView: Identifiable {
    public var id: FrequencyBand { band }
}

extension CandidateView: Identifiable {
    /// A dictionary entry is its dictionary, characters, and reading.
    public var id: String { [dictionary, simplified, traditional, pinyin].joined(separator: "\u{1F}") }
}

extension DictionaryEntryView: Identifiable {
    public var id: String { [dictionary, simplified, traditional, pinyin].joined(separator: "\u{1F}") }
}
