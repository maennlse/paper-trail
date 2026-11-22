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

    var body: some View {
        ZStack(alignment: .leading) {
            NavigationStack {
                VStack(spacing: 0) {
                    searchField
                    if let warning = model.storageWarning ?? model.snapshot?.unavailableFolders.first {
                        Button { showPreferences = true } label: {
                            Label(warning, systemImage: "exclamationmark.triangle")
                                .font(.caption).frame(maxWidth: .infinity, alignment: .leading)
                                .padding(10).background(.orange.opacity(0.15))
                        }
                        .buttonStyle(.plain)
                    }
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 12) {
                            let pinned = model.notes.filter(\.pinned)
                            if !pinned.isEmpty {
                                sectionTitle("Pinned")
                                ForEach(pinned, id: \.path) { note in
                                    Button { Task { await model.open(note.path) } } label: {
                                        PinnedNoteCard(note: note)
                                    }
                                    .buttonStyle(.plain)
                                    .contextMenu { noteActions(note) }
                                }
                            }
                            sectionTitle(model.query.isEmpty ? "Recent Notes" : "Search Results")
                            ForEach(model.notes.filter { !$0.pinned }, id: \.path) { note in
                                Button { Task { await model.open(note.path) } } label: {
                                    NoteRowView(note: note)
                                }
                                .buttonStyle(.plain)
                                .contextMenu { noteActions(note) }
                            }
                            if model.notes.isEmpty {
                                ContentUnavailableView(
                                    model.query.isEmpty ? "No Notes Yet" : "No Matching Notes",
                                    systemImage: model.query.isEmpty ? "note.text" : "magnifyingglass",
                                    description: Text(model.query.isEmpty ? "Compose a plain text note to get started." : "Search checks titles and full text across folders.")
                                )
                            }
                        }
                        .padding(.horizontal, 16)
                        .padding(.top, 16)
                        .padding(.bottom, 96)
                    }
                    .refreshable { await model.refresh() }
                }
                .background(Color(uiColor: .systemGroupedBackground))
                .navigationTitle(model.title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button { model.railOpen.toggle() } label: {
                            Image(systemName: "sidebar.left")
                        }
                        .accessibilityLabel("Show folders")
                        .accessibilityIdentifier("folder-toggle")
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button { showSync = true } label: {
                            Image(systemName: model.unlocked ? "lock.open" : "lock.fill")
                        }
                        .accessibilityLabel(model.unlocked ? "Sync vault unlocked" : "Sync vault locked or not configured")
                    }
                }
                .overlay(alignment: .bottomTrailing) {
                    Button { Task { await model.compose() } } label: {
                        Image(systemName: "square.and.pencil")
                            .font(.title2.weight(.semibold))
                            .foregroundStyle(.white)
                            .frame(width: 58, height: 58)
                            .background(.blue, in: RoundedRectangle(cornerRadius: 19))
                            .shadow(color: .blue.opacity(0.25), radius: 10, y: 4)
                    }
                    .accessibilityLabel("New note")
                    .accessibilityIdentifier("compose")
                    .disabled(model.snapshot == nil)
                    .padding(20)
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
                    preferences: { model.railOpen = false; showPreferences = true }
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
        .sheet(isPresented: $showSync) { SyncSheet() }
        .sheet(isPresented: $showPreferences) { PreferencesSheet() }
        .sheet(isPresented: $showNewFolder) { NewFolderSheet() }
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

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search across folders", text: $model.query)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityIdentifier("search")
            if !model.query.isEmpty {
                Button { model.query = "" } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }.accessibilityLabel("Clear search")
            }
        }
        .padding(10)
        .background(Color(uiColor: .tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10))
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }

    private func sectionTitle(_ text: String) -> some View {
        Text(text).font(.headline).padding(.top, 4).accessibilityAddTraits(.isHeader)
    }

    @ViewBuilder private func noteActions(_ note: NoteInfo) -> some View {
        Button(note.pinned ? "Unpin Note" : "Pin Note", systemImage: note.pinned ? "pin.slash" : "pin") {
            Task { await model.pin(note) }
        }
        Button("Delete Note", systemImage: "trash", role: .destructive) { pendingDelete = note }
    }
}

struct NoteRowView: View {
    let note: NoteInfo

    var body: some View {
        HStack(spacing: 10) {
            RoundedRectangle(cornerRadius: 2)
                .fill(Color(folderHex: note.color)).frame(width: 3)
            VStack(alignment: .leading, spacing: 7) {
                Text(note.title).font(.body.weight(.semibold))
                    .foregroundStyle(.primary).lineLimit(2)
                HStack(spacing: 10) {
                    Label {
                        Text(Date(timeIntervalSince1970: Double(note.modified)), style: .date)
                    } icon: { Image(systemName: "clock") }
                    Label(note.folderName, systemImage: "folder")
                        .foregroundStyle(Color(folderHex: note.color))
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    Text(".txt").font(.caption2.monospaced())
                        .padding(.horizontal, 5).padding(.vertical, 2)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: 4))
                }
                .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(uiColor: .secondarySystemGroupedBackground),
                    in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .combine)
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
