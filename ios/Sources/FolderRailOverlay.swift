// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct FolderRailOverlay: View {
    @EnvironmentObject private var model: LibraryModel
    let newFolder: () -> Void
    let sync: () -> Void
    let preferences: () -> Void

    var body: some View {
        VStack(spacing: 14) {
            ScrollView(showsIndicators: false) {
                VStack(spacing: 14) {
                    tile(name: "All", symbol: "tray.full.fill", color: .blue,
                         selected: model.selectedFolder == nil) { model.select(nil) }
                        .accessibilityLabel("All Notes")
                    ForEach(model.folders, id: \.path) { folder in
                        tile(name: String(folder.name.prefix(5)), symbol: "folder.fill",
                             color: Color(folderHex: folder.color),
                             selected: model.selectedFolder == folder.path) {
                            model.select(folder.path)
                        }
                        .accessibilityLabel(folder.name)
                    }
                    Button(action: newFolder) {
                        Image(systemName: "plus").font(.title2)
                            .frame(width: 48, height: 48)
                            .background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
                    }
                    .accessibilityLabel("New folder")
                }
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity)
            }
            Divider().padding(.horizontal, 12)
            railButton("icloud", label: "Sync", action: sync)
            railButton("gearshape", label: "Preferences", action: preferences)
        }
        .padding(.top, 8).padding(.bottom, 8)
        .frame(width: 76)
        .background(.regularMaterial, in: UnevenRoundedRectangle(
            bottomTrailingRadius: 20, topTrailingRadius: 20
        ))
        .shadow(color: .black.opacity(0.12), radius: 12, x: 4)
        .gesture(DragGesture(minimumDistance: 20).onEnded {
            if $0.translation.width < -30 { model.railOpen = false }
        })
        .accessibilityAddTraits(.isModal)
        .accessibilityAction(.escape) { model.railOpen = false }
    }

    private func tile(name: String, symbol: String, color: Color,
                      selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 2) {
                Image(systemName: symbol).font(.system(size: 17, weight: .semibold))
                Text(name).font(.system(size: 10, weight: .semibold))
                    .lineLimit(1).minimumScaleFactor(0.7)
            }
            .foregroundStyle(.primary)
            .frame(width: 48, height: 48)
            .background(color.opacity(0.28), in: RoundedRectangle(cornerRadius: 12))
            .overlay {
                if selected {
                    RoundedRectangle(cornerRadius: 14).stroke(.primary, lineWidth: 2).padding(-4)
                }
            }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }

    private func railButton(_ symbol: String, label: String,
                            action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol).font(.title3)
                .frame(width: 48, height: 44)
                .background(Color(uiColor: .secondarySystemGroupedBackground),
                            in: RoundedRectangle(cornerRadius: 12))
        }
        .accessibilityLabel(label)
    }
}

struct NewFolderSheet: View {
    private struct ColorChoice: Identifiable {
        let name: String
        let hex: String
        var id: String { hex }
    }

    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var color = "#2563c9"
    @State private var detent: PresentationDetent = .large
    private let colors = [
        ColorChoice(name: "Blue", hex: "#2563c9"),
        ColorChoice(name: "Sage", hex: "#779b82"),
        ColorChoice(name: "Amber", hex: "#d49b16"),
        ColorChoice(name: "Coral", hex: "#cd795e"),
        ColorChoice(name: "Violet", hex: "#6f5bd6"),
        ColorChoice(name: "Teal", hex: "#148085"),
    ]

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Folder name", text: $name)
                        .accessibilityIdentifier("new-folder-name")
                }
                Section("Color") {
                    ForEach(colors) { choice in
                        Button { color = choice.hex } label: {
                            HStack(spacing: 12) {
                                // Shapes retain their fill, unlike tinted SF Symbols
                                // rendered as monochrome native picker/menu images.
                                Circle()
                                    .fill(Color(folderHex: choice.hex))
                                    .frame(width: 24, height: 24)
                                    .overlay {
                                        Circle().strokeBorder(.primary.opacity(0.15), lineWidth: 1)
                                    }
                                    .accessibilityHidden(true)
                                Text(choice.name).foregroundStyle(.primary)
                                Spacer()
                                if color == choice.hex {
                                    Image(systemName: "checkmark")
                                        .foregroundStyle(Color.accentColor)
                                        .accessibilityHidden(true)
                                }
                            }
                            .frame(minHeight: 32)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(choice.name)
                        .accessibilityAddTraits(color == choice.hex ? [.isSelected] : [])
                        .accessibilityIdentifier("folder-color-\(choice.name.lowercased())")
                    }
                }
            }
            .navigationTitle("New Folder")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") {
                        Task {
                            await model.addFolder(name: name, color: color)
                            if model.errorMessage == nil { dismiss() }
                        }
                    }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .presentationDetents([.medium, .large], selection: $detent)
        .presentationDragIndicator(.visible)
        .alert("Paper Trail", isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.errorMessage = nil } }
        )) {
            Button("OK") { model.errorMessage = nil }
        } message: { Text(model.errorMessage ?? "") }
    }
}
