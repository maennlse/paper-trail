// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation
import XCTest

final class EditorStateTests: XCTestCase {
    func testAboutUsesBundleVersionAndBuildWithoutHardCoding() {
        XCTAssertEqual(AppVersion.description(info: [
            "CFBundleShortVersionString": "1.2.3", "CFBundleVersion": "42"
        ]), "1.2.3 (42)")
        XCTAssertEqual(AppVersion.description(info: ["CFBundleShortVersionString": "2.0"]), "2.0")
        XCTAssertEqual(AppVersion.description(info: [:]), "Unknown")
        XCTAssertEqual(AppVersion.sourceURL.absoluteString, "https://github.com/maennlse/paper-trail")
    }

    func testSyncAgeNeverShowsSeconds() {
        let synced = Date(timeIntervalSince1970: 1000)
        XCTAssertEqual(SyncRelativeTime.description(since: synced, now: synced), "Last synced: Just now")
        XCTAssertEqual(SyncRelativeTime.description(since: synced, now: synced.addingTimeInterval(59)),
                       "Last synced: Just now")
        for seconds: TimeInterval in [60, 119, 3600, 86400] {
            let text = SyncRelativeTime.description(since: synced, now: synced.addingTimeInterval(seconds))
            XCTAssertFalse(text.contains("second"))
            XCTAssertFalse(text.contains("Just now"))
        }
        XCTAssertEqual(SyncRelativeTime.description(since: synced, now: synced.addingTimeInterval(-10)),
                       "Last synced: Just now")
    }

    func testOlderSaveCannotMarkNewerEditsSavedOrFailed() {
        var state = EditorSaveState()
        state.changed()
        XCTAssertEqual(state.phase, .unsaved)
        let old = state.begin()
        XCTAssertEqual(state.phase, .saving)
        state.changed()
        state.complete(old)
        XCTAssertEqual(state.phase, .unsaved)
        state.fail(old, message: "Old failure")
        XCTAssertNil(state.failure)
        let current = state.begin()
        state.complete(current)
        XCTAssertEqual(state.phase, .saved)
    }

    func testFailureRetryAndNoteSwitchInvalidatePreviousCompletions() {
        var state = EditorSaveState()
        state.changed()
        let token = state.begin()
        state.fail(token, message: "Conflict")
        XCTAssertEqual(state.phase, .failed)
        XCTAssertEqual(state.failure, "Conflict")
        let retry = state.begin()
        XCTAssertEqual(state.phase, .saving)
        XCTAssertNil(state.failure)
        state.complete(retry)
        XCTAssertEqual(state.phase, .saved)
        state.reset()
        state.changed()
        state.complete(token)
        state.fail(token, message: "Previous note")
        XCTAssertEqual(state.phase, .unsaved)
    }

    func testLatestAttemptWinsEvenWhenTheTextRevisionIsUnchanged() {
        var state = EditorSaveState()
        state.changed()
        let autosave = state.begin()
        let flush = state.begin()
        state.complete(autosave)
        XCTAssertEqual(state.phase, .saving)
        state.fail(autosave, message: "Obsolete failure")
        XCTAssertNil(state.failure)
        state.complete(flush)
        state.fail(autosave, message: "Late obsolete failure")
        state.failPendingEdit(state.revision, message: "Late edit failure")
        XCTAssertEqual(state.phase, .saved)
        XCTAssertNil(state.failure)
    }

    func testSyncTimestampOnlyChangesOnUnqualifiedSuccess() {
        let previous = Date(timeIntervalSince1970: 100)
        let now = Date(timeIntervalSince1970: 200)
        var feedback = SyncFeedback(lastSuccess: previous)
        feedback.completed(at: now, succeeded: false, warning: "Offline", cancelled: false)
        XCTAssertEqual(feedback.lastSuccess, previous)
        XCTAssertEqual(feedback.failure, "Offline")
        feedback.completed(at: now, succeeded: true, warning: nil, cancelled: true)
        XCTAssertEqual(feedback.lastSuccess, previous)
        feedback.completed(at: now, succeeded: true, warning: "Partial sync", cancelled: false)
        XCTAssertEqual(feedback.lastSuccess, previous)
        feedback.completed(at: now, succeeded: true, warning: nil, cancelled: false)
        XCTAssertEqual(feedback.lastSuccess, now)
        XCTAssertNil(feedback.failure)
    }

    func testExportPreservesFilenameBytesAndCleansUpOnlyItsCopy() throws {
        let bytes = Data("Unsaved draft\nUnicode: café 🌿".utf8)
        var export: NoteExport? = try NoteExport(filename: "draft.md", data: bytes)
        let url = try XCTUnwrap(export?.url)
        XCTAssertEqual(url.lastPathComponent, "draft.md")
        try withExtendedLifetime(export) {
            XCTAssertEqual(try Data(contentsOf: url), bytes)
        }
        let other = try NoteExport(filename: "draft.md", data: Data([0, 255, 12]))
        XCTAssertNotEqual(other.url, url)
        export = nil
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
        XCTAssertEqual(try Data(contentsOf: other.url), Data([0, 255, 12]))
    }

    func testExportKeepsExtensionlessNamesAndCannotEscapeItsDirectory() throws {
        let export = try NoteExport(filename: "../README", data: Data())
        XCTAssertEqual(export.url.lastPathComponent, "README")
        XCTAssertTrue(export.url.deletingLastPathComponent().lastPathComponent
            .hasPrefix("papertrail-export-"))
    }
}
