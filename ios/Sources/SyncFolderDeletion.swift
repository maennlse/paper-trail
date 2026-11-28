// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

/// Capture the stable ID at confirmation time, never resolve duplicate names.
struct SyncFolderDeletion {
    let id: String
    let name: String

    init?(id: String?, name: String, published: Bool, deleted: Bool) {
        guard let id, !id.isEmpty, published, !deleted else { return nil }
        self.id = id
        self.name = name
    }

    static func isAvailable(signedIn: Bool, unlocked: Bool, busy: Bool, hasRecovery: Bool) -> Bool {
        signedIn && unlocked && !busy && !hasRecovery
    }

    var message: String {
        "Permanently delete “\(name)” and its note history from the server?\n\nFolder ID: \(id)\n\nLocal files and unsaved edits on all devices are kept. Other devices will stop syncing this folder when they next connect. Server backups are managed separately by the server operator."
    }
}
