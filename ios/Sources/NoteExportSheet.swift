// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct NoteExportSheet: View {
    @Environment(\.dismiss) private var dismiss
    let export: NoteExport

    var body: some View {
        NavigationStack {
            Form {
                Section("Exported Copy") {
                    Label(export.url.lastPathComponent, systemImage: "doc")
                    Text("This copy is not encrypted. Sharing can give other apps or people access to its contents. The original note is not changed.")
                        .font(.callout).foregroundStyle(.secondary)
                }
                Section {
                    // ShareLink supplies the native activity sheet and the
                    // correct iPad popover anchor; do not share a live note URL.
                    ShareLink(item: export.url) {
                        Label("Share File…", systemImage: "square.and.arrow.up")
                    }
                    .accessibilityIdentifier("share-export")
                }
            }
            .navigationTitle("Share Note")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
    }
}
