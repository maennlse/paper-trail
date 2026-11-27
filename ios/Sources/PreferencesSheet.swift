// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI
import UniformTypeIdentifiers
import UIKit

struct PreferencesSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var scale = 1.0
    @State private var appearance = "light"
    @State private var picker = false
    @State private var encryption = false
    @State private var editingFolder: FolderInfo?
    @State private var editor = EditorPreferences(
        showLineNumbers: true, wrapText: true, highlightCurrentLine: true,
        useMonospace: false, customFont: "")
    @State private var fontChoice = "system"
    @State private var previousFontChoice = "system"
    @State private var fontPicker = false
    @State private var loaded = false
    @State private var saving = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Editor") {
                    Toggle("Show Line Numbers", isOn: $editor.showLineNumbers)
                        .accessibilityIdentifier("editor-line-numbers")
                    Toggle("Wrap Text", isOn: $editor.wrapText)
                        .accessibilityIdentifier("editor-wrap-text")
                    Toggle("Highlight Current Line", isOn: $editor.highlightCurrentLine)
                        .accessibilityIdentifier("editor-highlight-current-line")
                    Text("Highlight Current Line is the setting formerly called “ruler”; it is not a vertical column guide.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                fontSection
                Section("Appearance") {
                    Picker("Appearance", selection: $appearance) {
                        Text("Light").tag("light")
                        Text("Dark").tag("dark")
                        Text("System").tag("system")
                    }
                }
                Section("Storage") {
                    Text(model.snapshot?.defaultFolder ?? "Documents/Paper Trail")
                        .font(.caption).textSelection(.enabled)
                    Button("Choose Folder in Files…") { picker = true }
                    Text("Choose a local or iCloud Drive folder. New notes are UTF-8 .txt files; existing text notes can use any extension.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Section("Registered Folders") {
                    ForEach(model.folders, id: \.path) { folder in
                        VStack(alignment: .leading) {
                            Text(folder.name)
                            Text(folder.path).font(.caption2).foregroundStyle(.secondary)
                            Button("Edit Folder…") { editingFolder = folder }
                            if model.folders.count > 1 {
                                Button("Disconnect Folder", role: .destructive) {
                                    Task { await model.forget(folder.path) }
                                }
                            }
                        }
                    }
                    Text("Disconnecting keeps files on disk and pauses sync for the folder.")
                        .font(.caption)
                }
                Section("Encryption") {
                    Button("Vault, Passphrase & Recovery") { encryption = true }
                    Text("Encryption protects sync data, not local note files. Vault locking does not hide notes stored on this device.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .disabled(saving)
            .navigationTitle("Preferences")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") {
                        var updated = editor
                        updated.customFont = fontChoice == "custom" ? editor.customFont : ""
                        updated.useMonospace = fontChoice == "monospace"
                        saving = true
                        Task {
                            defer { saving = false }
                            if await model.preferences(scale: scale, appearance: appearance, editor: updated) {
                                dismiss()
                            }
                        }
                    }
                    .disabled(saving || !loaded || (fontChoice == "custom" && editor.customFont.isEmpty))
                }
            }
            .onAppear { load() }
            .onChange(of: model.snapshot == nil) { _, _ in load() }
            .onChange(of: fontChoice) { previous, choice in
                if choice == "custom" && editor.customFont.isEmpty {
                    previousFontChoice = previous
                    fontPicker = true
                }
            }
            .sheet(isPresented: $fontPicker, onDismiss: {
                if fontChoice == "custom" && editor.customFont.isEmpty {
                    fontChoice = previousFontChoice
                }
            }) {
                EditorFontPicker(fontName: editor.customFont) { name in
                    if let name { editor.customFont = name }
                    fontPicker = false
                }
            }
            .sheet(isPresented: $picker) {
                FolderPicker { url in Task { await model.register(url) } }
            }
            .sheet(isPresented: $encryption) { SyncSheet() }
            .sheet(isPresented: Binding(
                get: { editingFolder != nil },
                set: { if !$0 { editingFolder = nil } }
            )) {
                if let folder = editingFolder { NewFolderSheet(folder: folder) }
            }
            .alert("Paper Trail", isPresented: Binding(
                get: { model.errorMessage != nil },
                set: { if !$0 { model.errorMessage = nil } }
            )) {
                Button("OK") { model.errorMessage = nil }
            } message: { Text(model.errorMessage ?? "") }
        }
        .interactiveDismissDisabled(saving)
    }

    private var fontSection: some View {
        Section("Editor Font") {
            Picker("Editor Font", selection: $fontChoice) {
                Text("System Default").tag("system")
                Text("Monospaced").tag("monospace")
                Text("Custom Font").tag("custom")
            }
            .accessibilityIdentifier("editor-font-choice")
            if fontChoice == "custom" {
                Button { fontPicker = true } label: {
                    LabeledContent("Choose Font…", value: editor.customFont.isEmpty
                                   ? "Not selected" : editor.customFont)
                }
                .accessibilityIdentifier("editor-custom-font")
                if !editor.customFont.isEmpty && UIFont(name: editor.customFont, size: 17) == nil {
                    Text("This font is unavailable on this device. The editor will use the system font until it is available again.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Slider(value: $scale, in: 0.7...2.5, step: 0.1) { Text("Font size") }
                .accessibilityIdentifier("editor-font-size")
            Text("Font size: \(Int(scale * 100))%")
            Text("The quick brown fox jumps over the lazy dog.\n0123456789")
                .font(Font(previewFont))
                .accessibilityIdentifier("editor-font-preview")
            Text("Font size also follows Dynamic Type. Custom fonts are chosen separately on iOS and desktop.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private var previewFont: UIFont {
        let size = CGFloat(17 * scale)
        let font: UIFont
        if fontChoice == "custom", let custom = UIFont(name: editor.customFont, size: size) {
            font = custom
        } else if fontChoice == "monospace" {
            font = .monospacedSystemFont(ofSize: size, weight: .regular)
        } else {
            font = .systemFont(ofSize: size)
        }
        return UIFontMetrics(forTextStyle: .body).scaledFont(for: font)
    }

    private func load() {
        guard !loaded, let snapshot = model.snapshot else { return }
        scale = snapshot.fontScale
        appearance = snapshot.appearance
        editor = snapshot.editorPreferences
        fontChoice = !editor.customFont.isEmpty ? "custom" : editor.useMonospace ? "monospace" : "system"
        loaded = true
    }
}

struct FolderPicker: UIViewControllerRepresentable {
    let selected: (URL) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(selected: selected) }
    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder], asCopy: false)
        picker.delegate = context.coordinator
        return picker
    }
    func updateUIViewController(_ controller: UIDocumentPickerViewController, context: Context) {}

    final class Coordinator: NSObject, UIDocumentPickerDelegate {
        let selected: (URL) -> Void
        init(selected: @escaping (URL) -> Void) { self.selected = selected }
        func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
            if let url = urls.first { selected(url) }
        }
    }
}
