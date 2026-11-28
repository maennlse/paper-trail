// SPDX-License-Identifier: GPL-3.0-or-later
import Combine
import Foundation
import Runestone

final class NoteFindState: ObservableObject {
    enum Method: String, CaseIterable, Identifiable {
        case contains = "Contains"
        case fullWord = "Matches Word"
        case startsWith = "Begins With"
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
    @Published var expanded = false
    @Published var showsReplacement = false
    @Published var replacement = ""
    @Published private(set) var matches: [NSRange] = []
    @Published private(set) var sourceText = ""
    @Published private(set) var selectedIndex = 0
    @Published private(set) var navigationVersion = 0
    // Installed by the live editor using weak references; never replace a SwiftUI
    // String wholesale, which would discard Runestone's undo/selection history.
    var replaceAction: ((Bool) -> Void)?

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

    func setResults(_ ranges: [NSRange], text: String = "") {
        sourceText = text
        matches = ranges
        selectedIndex = 0
    }

    func select(_ index: Int) {
        guard matches.indices.contains(index) else { return }
        selectedIndex = index
        navigationVersion += 1
    }

    func preview(at index: Int) -> String {
        guard matches.indices.contains(index) else { return "" }
        let text = sourceText as NSString
        let range = matches[index]
        guard range.location <= text.length, NSMaxRange(range) <= text.length else { return "" }
        // Limit work for very long single-line notes; preserve Unicode boundaries.
        let start = max(0, range.location - 40)
        let end = min(text.length, NSMaxRange(range) + 80)
        let context = text.rangeOfComposedCharacterSequences(
            for: NSRange(location: start, length: end - start))
        return text.substring(with: context).replacingOccurrences(of: "\n", with: " ")
    }

    private func replacementEdits(in text: String, ranges: [NSRange]) -> [(NSRange, String)] {
        guard method == .regularExpression else { return ranges.map { ($0, replacement) } }
        guard let expression = try? NSRegularExpression(
            pattern: query, options: caseSensitive ? [] : [.caseInsensitive])
        else { return [] }
        let substitutions = Dictionary(uniqueKeysWithValues: expression.matches(
            in: text, range: NSRange(location: 0, length: (text as NSString).length)).map {
                ($0.range, expression.replacementString(for: $0, in: text, offset: 0, template: replacement))
            })
        return ranges.compactMap { range in substitutions[range].map { (range, $0) } }
    }

    @discardableResult
    func replace(in view: Runestone.TextView, all: Bool) -> Bool {
        guard view.isEditable, isPresented, view.markedTextRange == nil,
              let query = searchQuery else { return false }
        let document = view.text
        let current = view.search(for: query).map(\.range)
        // A tap on stale results must never edit unrelated text.
        guard document == sourceText, current == matches else { return false }
        let targets = all ? current : current.enumerated().compactMap {
            $0.offset == selectedIndex ? $0.element : nil
        }
        let edits = replacementEdits(in: document, ranges: targets)
        guard !edits.isEmpty else { return false }
        if all {
            view.replaceText(in: BatchReplaceSet(replacements: edits.map {
                BatchReplaceSet.Replacement(range: $0.0, text: $0.1)
            }))
        } else if let (range, text) = edits.first {
            view.undoManager?.endUndoGrouping()
            view.undoManager?.beginUndoGrouping()
            view.replace(range, withText: text)
            view.undoManager?.setActionName("Replace")
            view.undoManager?.endUndoGrouping()
        }
        return true
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
