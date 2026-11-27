// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct NoteLanguageSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var saving = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    choice("Automatic", id: nil, selected: model.editorSyntax?.overrideId == nil)
                    ForEach(model.syntaxLanguages, id: \.id) { language in
                        choice(language.name, id: language.id,
                               selected: model.editorSyntax?.overrideId != nil &&
                                   model.editorSyntax?.unsupported == false &&
                                   model.editorSyntax?.languageId == language.id)
                    }
                } footer: {
                    Text("Automatic uses the filename extension. Plain Text disables highlighting. This changes display only, not the note’s contents.")
                }
                if model.editorSyntax?.unsupported == true {
                    Section {
                        Text("The saved language “\(model.editorSyntax?.overrideId ?? "")” is not bundled on iOS. It is preserved; this device displays the note as plain text.")
                    }
                }
            }
            .disabled(saving)
            .navigationTitle("Language")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }.disabled(saving)
                }
            }
            .alert("Could Not Change Language", isPresented: Binding(
                get: { model.errorMessage != nil },
                set: { if !$0 { model.errorMessage = nil } }
            )) {
                Button("OK") { model.errorMessage = nil }
            } message: { Text(model.errorMessage ?? "") }
        }
        .interactiveDismissDisabled(saving)
    }

    private func choice(_ name: String, id: String?, selected: Bool) -> some View {
        Button {
            saving = true
            Task {
                if await model.setEditorLanguage(id) { dismiss() }
                saving = false
            }
        } label: {
            HStack {
                Text(name).foregroundStyle(.primary)
                Spacer()
                if selected { Image(systemName: "checkmark").accessibilityHidden(true) }
            }
        }
        .accessibilityIdentifier("note-language-\(id ?? "automatic")")
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }
}
