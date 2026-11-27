// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct FolderRailOverlay: View {
    @EnvironmentObject private var model: LibraryModel
    let newFolder: () -> Void
    let sync: () -> Void
    let editFolder: (FolderInfo) -> Void
    @State private var dropTarget: String?

    var body: some View {
        VStack(spacing: 14) {
            ScrollView(showsIndicators: false) {
                VStack(spacing: 14) {
                    tile(badge: "All", color: .blue,
                         selected: model.selectedFolder == nil) { model.select(nil) }
                        .accessibilityLabel("All Notes")
                    ForEach(model.folders, id: \.path) { folder in
                        tile(badge: folder.badge,
                             color: Color(folderHex: folder.color),
                             selected: model.selectedFolder == folder.path) {
                            model.select(folder.path)
                        }
                        .accessibilityLabel(folder.name)
                        .draggable(folder.path)
                        .dropDestination(for: String.self) { paths, location in
                            guard paths.count == 1, let source = paths.first,
                                  source != folder.path,
                                  model.folders.contains(where: { $0.path == source }) else { return false }
                            Task {
                                await model.reorderFolder(source, relativeTo: folder.path,
                                                          after: location.y >= 24)
                            }
                            return true
                        } isTargeted: { targeted in
                            dropTarget = targeted ? folder.path : nil
                        }
                        .overlay {
                            if dropTarget == folder.path {
                                RoundedRectangle(cornerRadius: 10)
                                    .stroke(Color.accentColor, lineWidth: 2)
                                    .allowsHitTesting(false)
                            }
                        }
                        .contextMenu {
                            Button("Edit Folder…", systemImage: "pencil") { editFolder(folder) }
                            Button("Move Up", systemImage: "arrow.up") {
                                Task { await model.moveFolder(folder.path, by: -1) }
                            }.disabled(model.folders.first?.path == folder.path)
                            Button("Move Down", systemImage: "arrow.down") {
                                Task { await model.moveFolder(folder.path, by: 1) }
                            }.disabled(model.folders.last?.path == folder.path)
                        }
                        .accessibilityAction(named: "Edit Folder") { editFolder(folder) }
                        .accessibilityAction(named: "Move Up") {
                            Task { await model.moveFolder(folder.path, by: -1) }
                        }
                        .accessibilityAction(named: "Move Down") {
                            Task { await model.moveFolder(folder.path, by: 1) }
                        }
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
            Menu {
                Button("Sync Now", systemImage: "arrow.clockwise") {
                    Task { await model.sync(.synchronize) }
                }.disabled(model.busy || !model.unlocked)
                Button("Sync Settings", systemImage: "gearshape", action: sync)
            } label: {
                SyncReloadIcon(active: model.syncing)
                    .font(.title3)
                    .frame(width: 48, height: 44)
                    .background(Color(uiColor: .secondarySystemGroupedBackground),
                                in: RoundedRectangle(cornerRadius: 12))
            }
            .accessibilityLabel("Sync")
            .accessibilityValue(model.busy ? "Operation running" : model.unlocked ? "Ready" : "Setup or unlock required")
        }
        .padding(.top, 8).padding(.bottom, 8)
        .frame(width: 76)
        .background(.regularMaterial, in: UnevenRoundedRectangle(
            bottomTrailingRadius: 20, topTrailingRadius: 20
        ))
        .shadow(color: .black.opacity(0.12), radius: 12, x: 4)
        .accessibilityAddTraits(.isModal)
        .accessibilityAction(.escape) { model.railOpen = false }
    }

    private func tile(badge: String, color: Color,
                      selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            ZStack {
                Image(systemName: "folder.fill")
                    .font(.system(size: 38))
                    .foregroundStyle(color)
                Text(badge)
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(.white)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                    .frame(width: 30)
                    .offset(y: 3)
            }
            .frame(width: 48, height: 48)
            .background(selected ? color.opacity(0.15) : .clear,
                        in: RoundedRectangle(cornerRadius: 10))
            .overlay {
                if selected {
                    HStack {
                        Capsule().fill(color).frame(width: 3, height: 22)
                        Spacer()
                    }
                }
            }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }

}

private struct SyncReloadIcon: View {
    let active: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: !active || reduceMotion)) { context in
            Image(systemName: "arrow.clockwise")
                .rotationEffect(.degrees(active && !reduceMotion
                    ? context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1) * 360
                    : 0))
        }
        .accessibilityHidden(true)
    }
}

struct NewFolderSheet: View {
    var folder: FolderInfo? = nil
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
    @State private var saving = false
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
                    if !colors.contains(where: { $0.hex == color }) {
                        Label {
                            Text("Current color (\(color))")
                        } icon: {
                            Circle().fill(Color(folderHex: color)).frame(width: 24, height: 24)
                        }
                    }
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
            .disabled(saving)
            .navigationTitle(folder == nil ? "New Folder" : "Edit Folder")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(folder == nil ? "Create" : "Save") {
                        saving = true
                        Task {
                            defer { saving = false }
                            if let folder {
                                if await model.editFolder(folder, name: name, color: color) { dismiss() }
                            } else {
                                await model.addFolder(name: name, color: color)
                                if model.errorMessage == nil { dismiss() }
                            }
                        }
                    }.disabled(saving || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .onAppear {
                if let folder {
                    name = folder.name
                    color = folder.color
                }
            }
            .safeAreaInset(edge: .bottom) {
                if folder != nil {
                    Text("Changes the display name and color only. Files folders and bookmarks are not renamed.")
                        .font(.caption).foregroundStyle(.secondary).padding()
                }
            }
        }
        .interactiveDismissDisabled(saving)
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
