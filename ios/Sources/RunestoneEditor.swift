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
    @Environment(\.isEnabled) private var isEnabled
    @ScaledMetric(relativeTo: .body) private var baseFontSize: CGFloat = 17

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeUIView(context: Context) -> Runestone.TextView {
        let view = Runestone.TextView()
        view.text = text
        view.editorDelegate = context.coordinator
        view.showLineNumbers = false
        view.isLineWrappingEnabled = true
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
        if view.text != text, view.markedTextRange == nil {
            let selection = view.selectedRange
            view.text = text
            let location = min(selection.location, (text as NSString).length)
            view.selectedRange = NSRange(location: location,
                                         length: min(selection.length, (text as NSString).length - location))
        }
        let font = UIFont.systemFont(ofSize: baseFontSize * CGFloat(fontScale))
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
    let gutterBackgroundColor = UIColor.systemBackground
    let gutterHairlineColor = UIColor.separator
    let lineNumberColor = UIColor.secondaryLabel
    var lineNumberFont: UIFont { font }
    let selectedLineBackgroundColor = UIColor.clear
    let selectedLinesLineNumberColor = UIColor.label
    let selectedLinesGutterBackgroundColor = UIColor.systemBackground
    let invisibleCharactersColor = UIColor.tertiaryLabel
    let pageGuideHairlineColor = UIColor.separator
    let pageGuideBackgroundColor = UIColor.clear
    let markedTextBackgroundColor = UIColor.systemFill
    func textColor(for highlightName: String) -> UIColor? { nil }
}
