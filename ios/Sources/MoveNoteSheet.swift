// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct MoveNoteSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    let note: NoteInfo
    @State private var moving = false

    var body: some View {
        NavigationStack {
            List {
                Section("Choose a Destination") {
                    ForEach(model.folders.filter { $0.path != note.folder }, id: \.path) { folder in
                        Button {
                            moving = true
                            Task {
                                if await model.move(note, to: folder.path) { dismiss() }
                                moving = false
                            }
                        } label: {
                            Label(folder.name, systemImage: "folder.fill")
                                .foregroundStyle(Color(folderHex: folder.color))
                        }
                    }
                }
                Section {
                    Text("The note keeps its contents and pin. If the filename is already in use, a unique name is chosen without overwriting either file.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .disabled(moving)
            .overlay { if moving { ProgressView("Moving…") } }
            .navigationTitle("Move Note")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(moving)
                }
            }
        }
        .interactiveDismissDisabled(moving)
        .alert("Could Not Move", isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.errorMessage = nil } }
        )) {
            Button("OK") { model.errorMessage = nil }
        } message: { Text(model.errorMessage ?? "") }
    }
}
