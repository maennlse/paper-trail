// SPDX-License-Identifier: GPL-3.0-or-later
import Runestone
import SwiftUI
import UIKit

/// Keep the native editor alive across autosave updates so selection, marked
/// text, scrolling, and the undo stack are not reset on every keystroke.
struct RunestoneEditor: UIViewRepresentable {
    @Binding var text: String
    let fontScale: Double
    var findRequest = 0
    var preferences: EditorPreferences? = nil
    var languageID = "plain"
    @Environment(\.isEnabled) private var isEnabled
    @ScaledMetric(relativeTo: .body) private var baseFontSize: CGFloat = 17

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeUIView(context: Context) -> Runestone.TextView {
        let view = Runestone.TextView()
        view.text = text
        view.editorDelegate = context.coordinator
        view.isFindInteractionEnabled = true
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
        if context.coordinator.findRequest != findRequest {
            context.coordinator.findRequest = findRequest
            DispatchQueue.main.async { [weak view] in
                view?.findInteraction?.presentFindNavigator(showingReplace: false)
            }
        }
    }

    static func dismantleUIView(_ view: Runestone.TextView, coordinator: Coordinator) {
        view.editorDelegate = nil
    }

    final class Coordinator: TextViewDelegate {
        var text: Binding<String>
        var findRequest = 0
        var highlightCurrentLine: Bool?
        var languageID: String?
        let highlighter = SyntaxHighlighter()
        init(text: Binding<String>) { self.text = text }
        func textViewDidChange(_ textView: Runestone.TextView) {
            text.wrappedValue = textView.text
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
