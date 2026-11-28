// SPDX-License-Identifier: GPL-3.0-or-later
import Runestone
import SwiftUI
import UIKit

/// Keep the native editor alive across autosave updates so selection, marked
/// text, scrolling, and the undo stack are not reset on every keystroke.
struct RunestoneEditor: UIViewRepresentable {
    @Binding var text: String
    let fontScale: Double
    @ObservedObject var find: NoteFindState
    var preferences: EditorPreferences? = nil
    var languageID = "plain"
    @Environment(\.isEnabled) private var isEnabled
    @ScaledMetric(relativeTo: .body) private var baseFontSize: CGFloat = 17

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeUIView(context: Context) -> Runestone.TextView {
        let view = Runestone.TextView()
        view.text = text
        view.editorDelegate = context.coordinator
        // The floating panel uses Runestone's public search/highlight APIs,
        // not UIKit's inline find accessory.
        view.isFindInteractionEnabled = false
        view.smartQuotesType = .no
        view.smartDashesType = .no
        view.textContainerInset = UIEdgeInsets(top: 12, left: 16, bottom: 24, right: 16)
        view.backgroundColor = .systemBackground
        view.accessibilityIdentifier = "note-editor"
        view.accessibilityLabel = "Note text"
        return view
    }

    func updateUIView(_ view: Runestone.TextView, context: Context) {
        context.coordinator.text = $text
        view.isEditable = isEnabled
        if context.coordinator.languageID != languageID {
            context.coordinator.languageID = languageID
            view.setLanguageMode(context.coordinator.highlighter.mode(for: languageID))
        }
        let lineNumbers = preferences?.showLineNumbers ?? true
        let wrap = preferences?.wrapText ?? true
        let highlight = preferences?.highlightCurrentLine ?? true
        if view.showLineNumbers != lineNumbers { view.showLineNumbers = lineNumbers }
        if view.isLineWrappingEnabled != wrap { view.isLineWrappingEnabled = wrap }
        if context.coordinator.highlightCurrentLine != highlight {
            context.coordinator.highlightCurrentLine = highlight
            view.lineSelectionDisplayType = highlight ? .line : .disabled
        }
        if view.text != text, view.markedTextRange == nil {
            let selection = view.selectedRange
            view.text = text
            let location = min(selection.location, (text as NSString).length)
            view.selectedRange = NSRange(location: location,
                                         length: min(selection.length, (text as NSString).length - location))
        }
        let size = baseFontSize * CGFloat(fontScale)
        let custom = preferences?.customFont ?? ""
        let font = UIFont(name: custom, size: size) ?? ((preferences?.useMonospace ?? false)
            ? UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
            : UIFont.systemFont(ofSize: size))
        if view.theme.font != font {
            view.theme = NoteEditorTheme(font: font)
        }
        context.coordinator.updateFind(in: view, state: find)
    }

    static func dismantleUIView(_ view: Runestone.TextView, coordinator: Coordinator) {
        view.editorDelegate = nil
    }

    final class Coordinator: TextViewDelegate {
        var text: Binding<String>
        private var finding = false
        private var originalSelection = NSRange(location: 0, length: 0)
        private var lastQuery: SearchQuery?
        private var lastText: String?
        private var ranges: [NSRange] = []
        private var selectedIndex = 0
        private var navigationVersion = 0
        var highlightCurrentLine: Bool?
        var languageID: String?
        let highlighter = SyntaxHighlighter()
        init(text: Binding<String>) { self.text = text }
        func textViewDidChange(_ textView: Runestone.TextView) {
            text.wrappedValue = textView.text
        }

        func updateFind(in view: Runestone.TextView, state: NoteFindState) {
            state.replaceAction = { [weak self, weak view, weak state] all in
                guard let self, let view, let state, state.replace(in: view, all: all) else { return }
                self.text.wrappedValue = view.text
                self.updateFind(in: view, state: state)
            }
            guard state.isPresented else {
                if finding {
                    view.highlightedRanges = []
                    let length = view.text.utf16.count
                    let location = min(originalSelection.location, length)
                    view.selectedRange = NSRange(location: location,
                        length: min(originalSelection.length, length - location))
                    finding = false
                    lastText = nil
                    lastQuery = nil
                    DispatchQueue.main.async { [weak view] in _ = view?.becomeFirstResponder() }
                }
                return
            }
            if !finding {
                finding = true
                originalSelection = view.selectedRange
            }
            let query = state.searchQuery
            let text = view.text
            if query != lastQuery || text != lastText {
                lastQuery = query
                lastText = text
                ranges = query.map { view.search(for: $0).map(\.range) } ?? []
                selectedIndex = 0
                navigationVersion = state.navigationVersion
                showMatches(in: view)
                let results = ranges
                // Publish after UIViewRepresentable's update, and reject stale
                // results if the query, text, or presentation has since changed.
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.finding, state.isPresented,
                          self.lastQuery == query, self.lastText == text,
                          state.searchQuery == query else { return }
                    state.setResults(results, text: text)
                }
            } else if selectedIndex != state.selectedIndex || navigationVersion != state.navigationVersion {
                selectedIndex = state.selectedIndex
                navigationVersion = state.navigationVersion
                showMatches(in: view)
            }
        }

        private func showMatches(in view: Runestone.TextView) {
            view.highlightedRanges = ranges.enumerated().map { index, range in
                HighlightedRange(id: "find-\(index)", range: range,
                    color: (index == selectedIndex ? UIColor.systemOrange : .systemYellow).withAlphaComponent(0.4),
                    cornerRadius: 3)
            }
            if ranges.indices.contains(selectedIndex) {
                let range = ranges[selectedIndex]
                view.selectedRange = range
                view.scrollRangeToVisible(range)
            }
        }
    }
}

private final class NoteEditorTheme: Runestone.Theme {
    let font: UIFont
    init(font: UIFont) { self.font = font }
    let textColor = UIColor.label
    let gutterBackgroundColor = UIColor.secondarySystemBackground
    let gutterHairlineColor = UIColor.separator
    let lineNumberColor = UIColor.secondaryLabel
    var lineNumberFont: UIFont { .monospacedDigitSystemFont(ofSize: font.pointSize, weight: .regular) }
    let selectedLineBackgroundColor = UIColor.secondarySystemBackground
    let selectedLinesLineNumberColor = UIColor.label
    let selectedLinesGutterBackgroundColor = UIColor.tertiarySystemFill
    let invisibleCharactersColor = UIColor.tertiaryLabel
    let pageGuideHairlineColor = UIColor.separator
    let pageGuideBackgroundColor = UIColor.clear
    let markedTextBackgroundColor = UIColor.systemFill
    func textColor(for highlightName: String) -> UIColor? {
        let name = highlightName.lowercased()
        if name == "none" { return .label }
        if name.hasPrefix("comment") { return .secondaryLabel }
        if name.hasPrefix("text.title") || name.hasPrefix("text.uri") ||
            name.hasPrefix("text.reference") { return .systemBlue }
        if name.hasPrefix("text.literal") { return .systemGreen }
        switch name.split(separator: ".").first {
        case "keyword", "include", "conditional", "repeat", "exception": return .systemPurple
        case "string", "character": return .systemGreen
        case "number", "float", "boolean", "constant": return .systemOrange
        case "function", "method", "constructor": return .systemBlue
        case "type", "namespace": return .systemTeal
        case "property", "field", "attribute": return .systemBrown
        case "punctuation", "operator": return .secondaryLabel
        default: return nil
        }
    }

    func fontTraits(for highlightName: String) -> FontTraits {
        if highlightName == "text.title" || highlightName == "text.strong" { return .bold }
        if highlightName == "text.emphasis" { return .italic }
        return []
    }
}
