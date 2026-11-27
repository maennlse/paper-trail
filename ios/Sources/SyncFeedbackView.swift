// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct SyncFeedbackView: View {
    @EnvironmentObject private var model: LibraryModel
    let settings: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if model.syncing {
                ProgressView("Syncing… Local saving is separate.")
                    .font(.caption)
            } else if let failure = model.syncFeedback.failure {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange).lineLimit(3)
                HStack {
                    if model.unlocked {
                        Button("Retry Sync") { Task { await model.sync(.synchronize) } }
                            .disabled(model.busy)
                    }
                    Button(!model.configured ? "Sync Settings" :
                            !model.signedIn ? "Sign In…" :
                            !model.unlocked ? "Unlock Vault…" : "Sync Settings",
                           action: settings)
                }
                .font(.caption)
            }
            if let date = model.syncFeedback.lastSuccess {
                TimelineView(.periodic(from: date, by: 60)) { context in
                    Text(SyncRelativeTime.description(since: date, now: context.date))
                }
                .font(.caption2).foregroundStyle(.secondary)
                .accessibilityIdentifier("last-successful-sync")
            } else if model.configured {
                Text("Not synced successfully on this device yet.")
                    .font(.caption2).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
    }
}
