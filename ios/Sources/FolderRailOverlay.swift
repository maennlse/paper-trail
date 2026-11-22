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
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var color = "#2563c9"
    private let colors = ["#2563c9", "#779b82", "#d49b16", "#cd795e", "#6f5bd6", "#148085"]

    var body: some View {
        NavigationStack {
            Form {
                TextField("Folder name", text: $name)
                Picker("Color", selection: $color) {
                    ForEach(Array(colors.enumerated()), id: \.element) { index, value in
                        Label(["Blue", "Sage", "Amber", "Coral", "Violet", "Teal"][index],
                              systemImage: "circle.fill")
                            .foregroundStyle(Color(folderHex: value)).tag(value)
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
        .presentationDetents([.medium])
        .alert("Paper Trail", isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.errorMessage = nil } }
        )) {
            Button("OK") { model.errorMessage = nil }
        } message: { Text(model.errorMessage ?? "") }
    }
}
