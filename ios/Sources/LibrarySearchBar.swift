// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

/// A floating search capsule, separate from folder and compose controls.
struct LibrarySearchBar: View {
    @Binding var text: String
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                .accessibilityHidden(true)
            TextField("Search", text: $text)
                .focused($focused)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
                .submitLabel(.search).onSubmit { focused = false }
                .accessibilityLabel("Search across folders")
                .accessibilityIdentifier("library-search")
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                        .frame(minWidth: 44, minHeight: 44)
                }
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.leading, 16).padding(.trailing, text.isEmpty ? 16 : 2)
        .frame(minHeight: 54)
        .controlGlass(radius: 30, interactive: true)
        .accessibilityAction(.escape) { focused = false }
    }
}
