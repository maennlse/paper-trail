// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct PlainTextEditorView: View {
    @EnvironmentObject private var model: LibraryModel
    @StateObject private var find = NoteFindState()
    @State private var export: NoteExport?
    @State private var showSync = false
    @State private var showLanguage = false
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
                    find: find,
                    preferences: model.snapshot?.editorPreferences,
                    languageID: model.editorSyntax?.languageId ?? "plain"
                )
                if !find.isPresented {
                    VStack(spacing: 4) {
                        HStack(spacing: 6) {
                            if model.editorSave.phase == .saving { ProgressView().controlSize(.mini) }
                            Text(model.editorSave.phase.rawValue)
                                .accessibilityIdentifier("editor-save-status")
                            Spacer()
                            Button { showLanguage = true } label: {
                                Label(model.editorSyntax?.name ?? "Plain Text", systemImage: "chevron.up.chevron.down")
                            }
                            .accessibilityLabel("Language: \(model.editorSyntax?.name ?? "Plain Text")")
                            .accessibilityIdentifier("note-language")
                            .disabled(model.closingEditor)
                        }
                        .font(.caption).foregroundStyle(model.saveFailed ? Color.orange : Color.secondary)
                        .padding(.horizontal).frame(minHeight: 44)
                        if model.configured || model.syncFeedback.failure != nil {
                            SyncFeedbackView(settings: { showSync = true })
                                .padding(.horizontal).padding(.bottom, 6)
                        }
                    }
                    .padding(.vertical, 4).controlGlass(radius: 24)
                    .frame(maxWidth: 620)
                    .padding(.horizontal, 16).padding(.bottom, 8)
                }
            }
            .disabled(model.closingEditor)
            .navigationTitle(model.editorTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    VStack(spacing: 2) {
                        Text(model.editorTitle).font(.headline).lineLimit(1)
                            .accessibilityIdentifier("editor-note-title")
                        Text(model.editorPath.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "")
                            .font(.caption).foregroundStyle(.secondary)
                            .lineLimit(1).truncationMode(.middle)
                            .accessibilityIdentifier("editor-note-filename")
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button {
                            find.isPresented = true
                        } label: { Label("Find", systemImage: "magnifyingglass") }
                        .keyboardShortcut("f", modifiers: .command)
                        .accessibilityIdentifier("find-in-note")
                        Button {
                            do { export = try model.exportEditor() }
                            catch { model.errorMessage = error.localizedDescription }
                        } label: { Label("Share", systemImage: "square.and.arrow.up") }
                        .accessibilityIdentifier("share-note")
                        Button("Language", systemImage: "textformat") { showLanguage = true }
                    } label: {
                        Image(systemName: "ellipsis")
                    }
                    .accessibilityLabel("Note actions")
                    .accessibilityIdentifier("note-actions")
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") { Task { await model.closeEditor() } }
                        .disabled(model.closingEditor)
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if find.isPresented { NoteFindPanel(state: find) }
            }
            .sheet(item: $export) { NoteExportSheet(export: $0).nativeSheet() }
            .sheet(isPresented: $showSync) { SyncSheet().nativeSheet() }
            .sheet(isPresented: $showLanguage) { NoteLanguageSheet().nativeSheet() }
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
        .overlay(alignment: .leading) {
            // Reserve only the native back-gesture edge, not the text canvas.
            Color.clear.frame(width: 18).contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 20).onEnded { value in
                    if value.translation.width > 75,
                       abs(value.translation.height) < value.translation.width,
                       !model.closingEditor {
                        Task { await model.closeEditor() }
                    }
                })
                .accessibilityHidden(true)
        }
        .interactiveDismissDisabled()
    }
}
