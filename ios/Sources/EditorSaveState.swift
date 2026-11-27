// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

/// Tokens span note switches as well as edits. Late completions must never
/// describe a newer document/revision as saved (or failed).
struct EditorSaveState {
    struct Token: Equatable {
        let revision: UInt64
        let attempt: UInt64
    }

    enum Phase: String {
        case saved = "Saved locally"
        case unsaved = "Unsaved changes"
        case saving = "Saving…"
        case failed = "Not saved"
    }

    private(set) var revision: UInt64 = 0
    private var savedRevision: UInt64 = 0
    private var nextAttempt: UInt64 = 0
    private var latestToken: Token?
    private var savingToken: Token?
    private(set) var failure: String?

    var phase: Phase {
        if failure != nil { return .failed }
        if savingToken?.revision == revision { return .saving }
        if savedRevision == revision { return .saved }
        return .unsaved
    }

    mutating func reset() {
        changed()
        savedRevision = revision
    }

    mutating func changed() {
        revision += 1
        latestToken = nil
        savingToken = nil
        failure = nil
    }

    mutating func begin() -> Token {
        nextAttempt += 1
        let token = Token(revision: revision, attempt: nextAttempt)
        latestToken = token
        savingToken = token
        failure = nil
        return token
    }

    mutating func complete(_ token: Token) {
        guard token.revision == revision, latestToken == token else { return }
        savedRevision = token.revision
        savingToken = nil
        failure = nil
    }

    mutating func fail(_ token: Token, message: String) {
        guard token.revision == revision, latestToken == token else { return }
        savingToken = nil
        failure = message
    }

    mutating func failPendingEdit(_ revision: UInt64, message: String) {
        guard revision == self.revision, latestToken == nil,
              savedRevision != revision else { return }
        failure = message
    }
}

struct SyncFeedback {
    var lastSuccess: Date?
    var failure: String?

    mutating func completed(at date: Date, succeeded: Bool, warning: String?, cancelled: Bool) {
        guard !cancelled else { return }
        failure = warning
        if succeeded && warning == nil { lastSuccess = date }
    }
}
