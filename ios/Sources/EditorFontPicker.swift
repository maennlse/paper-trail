// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI
import UIKit

struct EditorFontPicker: UIViewControllerRepresentable {
    let fontName: String
    let selected: (String?) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(selected: selected) }

    func makeUIViewController(context: Context) -> UIFontPickerViewController {
        let configuration = UIFontPickerViewController.Configuration()
        configuration.includeFaces = true
        let picker = UIFontPickerViewController(configuration: configuration)
        picker.delegate = context.coordinator
        picker.selectedFontDescriptor = UIFont(name: fontName, size: 17)?.fontDescriptor
        return picker
    }

    func updateUIViewController(_ controller: UIFontPickerViewController, context: Context) {
        context.coordinator.selected = selected
    }

    final class Coordinator: NSObject, UIFontPickerViewControllerDelegate {
        var selected: (String?) -> Void
        init(selected: @escaping (String?) -> Void) { self.selected = selected }
        func fontPickerViewControllerDidPickFont(_ viewController: UIFontPickerViewController) {
            selected(viewController.selectedFontDescriptor?.postscriptName)
        }
        func fontPickerViewControllerDidCancel(_ viewController: UIFontPickerViewController) {
            selected(nil)
        }
    }
}
