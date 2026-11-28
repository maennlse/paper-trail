// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct NoteFindPanel: View {
    @ObservedObject var state: NoteFindState
    @FocusState private var focused: Bool

    var body: some View {
        VStack(spacing: 8) {
            Capsule().fill(.tertiary).frame(width: 34, height: 4)
                .padding(.top, 2).accessibilityHidden(true)
                .gesture(DragGesture().onEnded { if $0.translation.height > 30 { close() } })
            HStack(spacing: 8) {
                HStack {
                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                    TextField("Find", text: $state.query)
                        .focused($focused).submitLabel(.search)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .onSubmit { state.move(1) }
                        .accessibilityIdentifier("note-find-query")
                }
                .padding(12)
                .background(Color(uiColor: .secondarySystemGroupedBackground),
                            in: RoundedRectangle(cornerRadius: 14))
                Menu {
                    Toggle("Match Case", isOn: $state.caseSensitive)
                    Picker("Match", selection: $state.method) {
                        ForEach(NoteFindState.Method.allCases) { method in
                            Text(method.rawValue).tag(method)
                        }
                    }
                } label: {
                    Image(systemName: "ellipsis.circle").font(.title2).frame(width: 44, height: 44)
                }
                .accessibilityLabel("Find Options").accessibilityIdentifier("note-find-options")
            }
            HStack {
                Button(action: close) { Image(systemName: "xmark").frame(width: 44, height: 44) }
                    .accessibilityLabel("Close Find").accessibilityIdentifier("note-find-close")
                Spacer()
                Text(state.validationMessage ?? state.countDescription)
                    .font(.caption).foregroundStyle(state.validationMessage == nil ? Color.secondary : .orange)
                    .accessibilityIdentifier("note-find-count")
                Spacer()
                Button { state.move(-1) } label: {
                    Image(systemName: "chevron.up").frame(width: 44, height: 44)
                }
                .accessibilityLabel("Previous Match").accessibilityIdentifier("note-find-previous")
                .disabled(state.matches.isEmpty)
                Button { state.move(1) } label: {
                    Image(systemName: "chevron.down").frame(width: 44, height: 44)
                }
                .accessibilityLabel("Next Match").accessibilityIdentifier("note-find-next")
                .disabled(state.matches.isEmpty)
            }
        }
        .padding(12).foregroundStyle(.primary)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 22))
        .overlay { RoundedRectangle(cornerRadius: 22).stroke(.quaternary) }
        .shadow(color: .black.opacity(0.12), radius: 12, y: 2)
        .padding(.horizontal, 16).padding(.bottom, 8)
        .task { focused = true }
        .accessibilityAction(.escape) { close() }
    }

    private func close() {
        focused = false
        state.isPresented = false
    }
}
