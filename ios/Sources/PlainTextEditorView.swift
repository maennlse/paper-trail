// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct PlainTextEditorView: View {
    @EnvironmentObject private var model: LibraryModel
    @State private var findRequest = 0
    @State private var export: NoteExport?
    @State private var showSync = false
    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if model.saveFailed {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("Not saved. Your edits are kept.", systemImage: "exclamationmark.triangle")
                        HStack {
                            Button("Retry") { Task { await model.retrySave() } }
                            Button("Save a Copy") { Task { await model.saveCopy() } }
                        }
                    }
                    .padding().background(.orange.opacity(0.15))
                    if let failure = model.editorSave.failure {
                        Text(failure).font(.caption).foregroundStyle(.secondary)
                            .padding(.horizontal)
                    }
                }
                RunestoneEditor(
                    text: Binding(get: { model.editorText }, set: { model.edited($0) }),
                    fontScale: model.snapshot?.fontScale ?? 1,
                    findRequest: findRequest
                )
                HStack(spacing: 6) {
                    if model.editorSave.phase == .saving { ProgressView().controlSize(.mini) }
                    Text(model.editorSave.phase.rawValue)
                        .accessibilityIdentifier("editor-save-status")
                    Spacer()
                }
                .font(.caption).foregroundStyle(model.saveFailed ? Color.orange : Color.secondary)
                .padding(.horizontal).padding(.vertical, 6)
                if model.configured || model.syncFeedback.failure != nil {
                    SyncFeedbackView(settings: { showSync = true })
                        .padding(.horizontal).padding(.bottom, 6)
                }
            }
            .disabled(model.closingEditor)
            .navigationTitle("Plain Text")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItemGroup(placement: .topBarLeading) {
                    Button {
                        findRequest += 1
                    } label: { Label("Find", systemImage: "magnifyingglass") }
                    .keyboardShortcut("f", modifiers: .command)
                    .accessibilityIdentifier("find-in-note")
                    Button {
                        do { export = try model.exportEditor() }
                        catch { model.errorMessage = error.localizedDescription }
                    } label: { Label("Share", systemImage: "square.and.arrow.up") }
                    .accessibilityIdentifier("share-note")
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { Task { await model.closeEditor() } }
                        .disabled(model.closingEditor)
                }
            }
            .sheet(item: $export) { NoteExportSheet(export: $0) }
            .sheet(isPresented: $showSync) { SyncSheet() }
            .alert("Paper Trail", isPresented: Binding(
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
