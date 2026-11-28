// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

extension Color {
    init(folderHex: String) {
        let value = UInt64(folderHex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) ?? 0x2563c9
        self.init(.sRGB, red: Double((value >> 16) & 255) / 255,
                  green: Double((value >> 8) & 255) / 255,
                  blue: Double(value & 255) / 255, opacity: 1)
    }
}

struct NoteListView: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var showSync = false
    @State private var showPreferences = false
    @State private var showNewFolder = false
    @State private var pendingDelete: NoteInfo?
    @State private var pendingRename: NoteInfo?
    @State private var pendingMove: NoteInfo?
    @State private var pendingFolderEdit: FolderInfo?
    @State private var export: NoteExport?
    @State private var exporting = false

    var body: some View {
        ZStack(alignment: .leading) {
            NavigationStack {
                VStack(spacing: 0) {
                    if model.syncing || model.syncFeedback.failure != nil {
                        SyncFeedbackView(settings: { showSync = true }, showLastSync: false)
                            .padding(.horizontal, 16).padding(.bottom, 8)
                    }
                    if let warning = model.storageWarning ?? model.snapshot?.unavailableFolders.first {
                        Button { showPreferences = true } label: {
                            Label(warning, systemImage: "exclamationmark.triangle")
                                .font(.caption).frame(maxWidth: .infinity, alignment: .leading)
                                .padding(10).background(.orange.opacity(0.15))
                        }
                        .buttonStyle(.plain)
                    }
                    List {
                        let pinned = model.notes.filter(\.pinned)
                        if !pinned.isEmpty {
                            Section("Pinned") {
                                ForEach(pinned, id: \.path) { note in noteButton(note) }
                            }
                        }
                        Section(model.query.isEmpty ? "Recent Notes" : "Search Results") {
                            ForEach(model.notes.filter { !$0.pinned }, id: \.path) { note in
                                noteButton(note)
                            }
                        }
                    }
                    .listStyle(.insetGrouped)
                    .overlay {
                        if model.notes.isEmpty {
                            ContentUnavailableView(
                                model.query.isEmpty ? "No Notes Yet" : "No Matching Notes",
                                systemImage: model.query.isEmpty ? "note.text" : "magnifyingglass",
                                description: Text(model.query.isEmpty ? "Compose a plain text note to get started." : "Search checks titles and full text across folders.")
                            )
                            .allowsHitTesting(false)
                        }
                    }
                    .refreshable { await model.refreshAndSync() }
                }
                .background(Color(uiColor: .systemGroupedBackground))
                .navigationTitle(model.title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button { showPreferences = true } label: {
                            Image(systemName: "gearshape")
                        }
                        .accessibilityLabel("Preferences")
                        .accessibilityIdentifier("preferences")
                    }
                }
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    VStack(spacing: 8) {
                        HStack(spacing: 10) {
                            Button { model.railOpen.toggle() } label: {
                                Image(systemName: "line.3.horizontal.decrease")
                                    .font(.title3.weight(.medium))
                                    .frame(width: 54, height: 54)
                            }
                            .buttonStyle(.plain)
                            .controlGlass(radius: 27, interactive: true)
                            .accessibilityLabel("Show folders")
                            .accessibilityIdentifier("folder-toggle")
                            LibrarySearchBar(text: $model.query)
                            Button { Task { await model.compose() } } label: {
                                Image(systemName: "plus").font(.title2.weight(.medium))
                                    .frame(width: 54, height: 54)
                            }
                            .buttonStyle(.plain)
                            .controlGlass(radius: 27, interactive: true)
                            .accessibilityLabel("New note").accessibilityIdentifier("compose")
                            .disabled(model.snapshot == nil)
                        }
                        .padding(.horizontal, 16)
                        .frame(maxWidth: 720)
                        if model.configured {
                            SyncFeedbackView(settings: { showSync = true }, showStatus: false)
                                .padding(.horizontal, 16).padding(.bottom, 6)
                        }
                    }
                    .padding(.top, 8).padding(.bottom, 6)
                }
            }
            .accessibilityHidden(model.railOpen)

            // Edge-only hit target does not interfere with scrolling or text selection.
            if !model.railOpen {
                Color.clear.frame(width: 18).contentShape(Rectangle())
                    .gesture(DragGesture(minimumDistance: 20).onEnded {
                        if $0.translation.width > 45,
                           abs($0.translation.height) < abs($0.translation.width) {
                            model.railOpen = true
                        }
                    })
                    .accessibilityHidden(true)
            }
            if model.railOpen {
                Color.black.opacity(0.25)
                    .ignoresSafeArea()
                    .onTapGesture { model.railOpen = false }
                    .gesture(DragGesture(minimumDistance: 20).onEnded {
                        if $0.translation.width < -30 { model.railOpen = false }
                    })
                    .accessibilityLabel("Dismiss folders")
                    .accessibilityAddTraits(.isButton)
                    .transition(.opacity)
                FolderRailOverlay(
                    newFolder: { showNewFolder = true },
                    sync: { model.railOpen = false; showSync = true },
                    editFolder: { folder in model.railOpen = false; pendingFolderEdit = folder }
                )
                .transition(.move(edge: .leading))
                .accessibilityIdentifier("folder-rail")
            }
        }
        .animation(reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.88), value: model.railOpen)
        .task(id: model.query) {
            do { try await Task.sleep(for: .milliseconds(150)) }
            catch { return }
            await model.refresh()
        }
        .sheet(isPresented: $showSync) { SyncSheet().nativeSheet() }
        .sheet(isPresented: $showPreferences) { PreferencesSheet().nativeSheet() }
        .sheet(isPresented: $showNewFolder) { NewFolderSheet() }
        .sheet(item: $export) { NoteExportSheet(export: $0).nativeSheet() }
        .sheet(isPresented: Binding(
            get: { pendingMove != nil },
            set: { if !$0 { pendingMove = nil } }
        )) {
            if let note = pendingMove { MoveNoteSheet(note: note) }
        }
        .sheet(isPresented: Binding(
            get: { pendingFolderEdit != nil },
            set: { if !$0 { pendingFolderEdit = nil } }
        )) {
            if let folder = pendingFolderEdit { NewFolderSheet(folder: folder) }
        }
        .sheet(isPresented: Binding(
            get: { pendingRename != nil },
            set: { if !$0 { pendingRename = nil } }
        )) {
            if let note = pendingRename { RenameNoteSheet(note: note) }
        }
        .fullScreenCover(isPresented: Binding(
            get: { model.editorPath != nil },
            set: { _ in } // Only explicit, successfully saved dismissal is allowed.
        )) { PlainTextEditorView() }
        .alert("Paper Trail", isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.errorMessage = nil } }
        )) {
            Button("OK") { model.errorMessage = nil }
        } message: { Text(model.errorMessage ?? "") }
        .confirmationDialog("Delete this note permanently?", isPresented: Binding(
            get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }
        ), titleVisibility: .visible) {
            if let note = pendingDelete {
                Button("Delete Note", role: .destructive) {
                    Task { await model.delete(note) }
                    pendingDelete = nil
                }
            }
        }
    }

    private func noteButton(_ note: NoteInfo) -> some View {
        Button { Task { await model.open(note.path) } } label: {
            if note.pinned { PinnedNoteCard(note: note) }
            else { NoteRowView(note: note) }
        }
        .buttonStyle(.plain)
        .listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 4, trailing: 0))
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .contextMenu { noteActions(note) }
        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
            // Keep the row until the native confirmation is accepted; a
            // destructive swipe button would optimistically animate it away.
            Button("Delete", systemImage: "trash") { pendingDelete = note }.tint(.red)
            Button("Rename", systemImage: "pencil") { pendingRename = note }
                .tint(.blue)
        }
        .swipeActions(edge: .leading, allowsFullSwipe: true) {
            Button(note.pinned ? "Unpin" : "Pin",
                   systemImage: note.pinned ? "pin.slash" : "pin") {
                Task { await model.pin(note) }
            }.tint(.orange)
        }
        .accessibilityAction(named: note.pinned ? "Unpin Note" : "Pin Note") {
            Task { await model.pin(note) }
        }
        .accessibilityAction(named: "Rename Note") { pendingRename = note }
        .accessibilityAction(named: "Move Note") {
            if model.folders.contains(where: { $0.path != note.folder }) { pendingMove = note }
        }
        .accessibilityAction(named: "Share Note") { share(note) }
        .accessibilityAction(named: "Delete Note") { pendingDelete = note }
    }

    @ViewBuilder private func noteActions(_ note: NoteInfo) -> some View {
        Button(note.pinned ? "Unpin Note" : "Pin Note", systemImage: note.pinned ? "pin.slash" : "pin") {
            Task { await model.pin(note) }
        }
        Button("Rename Note", systemImage: "pencil") { pendingRename = note }
        Button("Move…", systemImage: "folder") { pendingMove = note }
            .disabled(!model.folders.contains(where: { $0.path != note.folder }))
        Button("Share…", systemImage: "square.and.arrow.up") { share(note) }
            .disabled(exporting)
        Button("Delete Note", systemImage: "trash", role: .destructive) { pendingDelete = note }
    }

    private func share(_ note: NoteInfo) {
        guard !exporting else { return }
        exporting = true
        Task {
            defer { exporting = false }
            do { export = try await model.export(note) }
            catch { model.errorMessage = error.localizedDescription }
        }
    }
}

struct NoteRowView: View {
    let note: NoteInfo

    var body: some View {
        HStack(spacing: 10) {
            RoundedRectangle(cornerRadius: 2)
                .fill(Color(folderHex: note.color)).frame(width: 3)
            VStack(alignment: .leading, spacing: 7) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(note.title).font(.body.weight(.semibold))
                        .foregroundStyle(.primary).lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    TimelineView(.periodic(from: .now, by: 60)) { context in
                        Text(NoteTimestamp.description(
                            Date(timeIntervalSince1970: Double(note.modified)), now: context.date))
                            .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                            .fixedSize(horizontal: true, vertical: false)
                    }
                }
                Text(URL(fileURLWithPath: note.path).lastPathComponent)
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                HStack(spacing: 10) {
                    FolderBadge(badge: note.folderBadge, color: Color(folderHex: note.color), size: 25)
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("Folder: \(note.folderName)")
                    Spacer(minLength: 0)
                    Text(fileType).font(.caption2.monospaced())
                        .padding(.horizontal, 5).padding(.vertical, 2)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: 4))
                }
                .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private var fileType: String {
        let ext = URL(fileURLWithPath: note.path).pathExtension
        return ext.isEmpty ? "file" : ".\(ext)"
    }
}

struct RenameNoteSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    let note: NoteInfo
    @State private var filename = ""
    @State private var saving = false

    var body: some View {
        NavigationStack {
            Form {
                TextField("Filename", text: $filename)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("rename-filename")
                Text("Renaming changes the filename, not the note’s first line.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .navigationTitle("Rename Note")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Rename") {
                        saving = true
                        Task {
                            if await model.rename(note, filename: filename) { dismiss() }
                            saving = false
                        }
                    }.disabled(saving || filename.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .onAppear { filename = URL(fileURLWithPath: note.path).lastPathComponent }
        .interactiveDismissDisabled(saving)
        .alert("Could Not Rename", isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.errorMessage = nil } }
        )) {
            Button("OK") { model.errorMessage = nil }
        } message: { Text(model.errorMessage ?? "") }
    }
}

struct PinnedNoteCard: View {
    let note: NoteInfo
    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "pin.fill")
                .foregroundStyle(Color(folderHex: note.color))
                .accessibilityHidden(true)
            NoteRowView(note: note)
        }
    }
}
