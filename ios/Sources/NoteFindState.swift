// SPDX-License-Identifier: GPL-3.0-or-later
import Combine
import Foundation
import Runestone

final class NoteFindState: ObservableObject {
    enum Method: String, CaseIterable, Identifiable {
        case contains = "Contains"
        case fullWord = "Whole Words"
        case startsWith = "Starts With"
        case endsWith = "Ends With"
        case regularExpression = "Regular Expression"
        var id: Self { self }
        var matchMethod: SearchQuery.MatchMethod {
            switch self {
            case .contains: return .contains
            case .fullWord: return .fullWord
            case .startsWith: return .startsWith
            case .endsWith: return .endsWith
            case .regularExpression: return .regularExpression
            }
        }
    }

    @Published var isPresented = false
    @Published var query = "" { didSet { if query != oldValue { clearResults() } } }
    @Published var caseSensitive = false { didSet { if caseSensitive != oldValue { clearResults() } } }
    @Published var method: Method = .contains { didSet { if method != oldValue { clearResults() } } }
    @Published private(set) var matches: [NSRange] = []
    @Published private(set) var selectedIndex = 0
    @Published private(set) var navigationVersion = 0

    var validationMessage: String? {
        guard method == .regularExpression, !query.isEmpty else { return nil }
        return (try? NSRegularExpression(pattern: query)) == nil ? "Invalid regular expression" : nil
    }

    var searchQuery: SearchQuery? {
        guard isPresented, !query.isEmpty, validationMessage == nil else { return nil }
        return SearchQuery(text: query, matchMethod: method.matchMethod, isCaseSensitive: caseSensitive)
    }

    var countDescription: String {
        matches.isEmpty ? "0 matches" : "\(selectedIndex + 1) of \(matches.count)"
    }

    func setResults(_ ranges: [NSRange]) {
        matches = ranges
        selectedIndex = 0
    }

    func move(_ delta: Int) {
        guard !matches.isEmpty else { return }
        selectedIndex = (selectedIndex + delta % matches.count + matches.count) % matches.count
        navigationVersion += 1
    }

    private func clearResults() {
        matches = []
        selectedIndex = 0
    }
}
