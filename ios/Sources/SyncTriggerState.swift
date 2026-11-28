// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

/// Coalesce close/pull requests behind an in-flight operation; never overlap sync.
struct SyncTriggerState {
    private(set) var pending = false
    mutating func request() { pending = true }
    mutating func cancel() { pending = false }
    mutating func takeIfReady(allowed: Bool, busy: Bool) -> Bool {
        guard allowed, !busy, pending else { return false }
        pending = false
        return true
    }
}
