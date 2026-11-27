// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI
import UniformTypeIdentifiers

struct PreferencesSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var scale = 1.0
    @State private var appearance = "light"
    @State private var picker = false
    @State private var encryption = false
    @State private var editingFolder: FolderInfo?

    var body: some View {
        NavigationStack {
            Form {
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
                Section("Editor") {
                    Slider(value: $scale, in: 0.7...2.5, step: 0.1) { Text("Font size") }
                    Text("Font size: \(Int(scale * 100))%")
                    Picker("Appearance", selection: $appearance) {
                        Text("Light").tag("light")
                        Text("Dark").tag("dark")
                        Text("System").tag("system")
                    }
                }
                Section("Encryption") {
                    Button("Vault, Passphrase & Recovery") { encryption = true }
                    Text("Encryption protects sync data, not local note files. Vault locking does not hide notes stored on this device.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Preferences")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") {
                        Task {
                            await model.preferences(scale: scale, appearance: appearance)
                            if model.errorMessage == nil { dismiss() }
                        }
                    }
                }
            }
            .onAppear {
                scale = model.snapshot?.fontScale ?? 1
                appearance = model.snapshot?.appearance ?? "light"
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
