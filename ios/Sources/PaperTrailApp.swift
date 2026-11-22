// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

@main
struct PaperTrailApp: App {
    @StateObject private var model = LibraryModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            NoteListView()
                .environmentObject(model)
                .preferredColorScheme(model.colorScheme)
                .task { await model.start() }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .active {
                        Task { await model.refresh() }
                    } else {
                        model.flushForBackground()
                    }
                }
        }
    }
}
