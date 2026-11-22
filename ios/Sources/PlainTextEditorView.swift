// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct PlainTextEditorView: View {
    @EnvironmentObject private var model: LibraryModel
    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if model.saveFailed {
                    HStack {
                        Label("Not saved. Your edits are kept.", systemImage: "exclamationmark.triangle")
                            .font(.caption)
                        Spacer()
                        Button("Save a Copy") { Task { await model.saveCopy() } }
                    }
                    .padding().background(.orange.opacity(0.15))
                }
                TextEditor(text: Binding(get: { model.editorText }, set: { model.edited($0) }))
                    .font(.system(size: 17 * (model.snapshot?.fontScale ?? 1)))
                    .padding(.horizontal, 8)
                    .accessibilityIdentifier("note-editor")
            }
            .navigationTitle("Plain Text")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { Task { await model.closeEditor() } }
                }
            }
            .alert("Could Not Save", isPresented: Binding(
                get: { model.errorMessage != nil },
                set: { if !$0 { model.errorMessage = nil } }
            )) {
                Button("OK") { model.errorMessage = nil }
                if model.saveFailed {
                    Button("Save a Copy") {
                        model.errorMessage = nil
                        Task { await model.saveCopy() }
                    }
                }
            } message: { Text(model.errorMessage ?? "") }
        }
        .interactiveDismissDisabled()
    }
}
