// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation
import Runestone
import XCTest

final class NoteExperienceTests: XCTestCase {
    func testTimestampMatchesGTKAndUsesTheLocalCalendar() throws {
        let iso = ISO8601DateFormatter()
        let now = try XCTUnwrap(iso.date(from: "2026-11-28T14:57:00Z"))
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        let locale = Locale(identifier: "en_US_POSIX")
        for (date, expected) in [
            ("2026-11-28T09:05:00Z", "09:05"),
            ("2026-11-02T09:05:00Z", "Nov 02"),
            ("2025-11-28T09:05:00Z", "2025-11-28")
        ] {
            XCTAssertEqual(NoteTimestamp.description(try XCTUnwrap(iso.date(from: date)),
                now: now, calendar: calendar, locale: locale), expected)
        }
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        XCTAssertEqual(NoteTimestamp.description(
            try XCTUnwrap(iso.date(from: "2026-11-28T01:00:00Z")),
            now: now, calendar: calendar, locale: locale), "Nov 27")
    }

    func testSyncRequestsCoalesceWaitForReadinessAndRespectCancellation() {
        var state = SyncTriggerState()
        state.request()
        state.request()
        XCTAssertFalse(state.takeIfReady(allowed: true, busy: true))
        XCTAssertTrue(state.pending)
        XCTAssertFalse(state.takeIfReady(allowed: false, busy: false))
        XCTAssertTrue(state.takeIfReady(allowed: true, busy: false))
        XCTAssertFalse(state.takeIfReady(allowed: true, busy: false))
        state.request()
        state.cancel()
        XCTAssertFalse(state.takeIfReady(allowed: true, busy: false))
    }

    @MainActor
    func testFindUsesRunestoneMatchesAndWrapsWithoutChangingTheDocument() throws {
        let state = NoteFindState()
        let view = Runestone.TextView()
        let text = "🌿 café CAFÉ\nwalk walking walk"
        view.text = text
        let selection = NSRange(location: 3, length: 4)
        view.selectedRange = selection
        state.isPresented = true
        state.query = "café"
        state.setResults(view.search(for: try XCTUnwrap(state.searchQuery)).map(\.range))
        XCTAssertEqual(state.matches.count, 2)
        XCTAssertEqual(state.matches.first, NSRange(location: 3, length: 4))
        state.move(-1)
        XCTAssertEqual(state.countDescription, "2 of 2")
        state.move(1)
        XCTAssertEqual(state.countDescription, "1 of 2")
        state.caseSensitive = true
        state.setResults(view.search(for: try XCTUnwrap(state.searchQuery)).map(\.range))
        XCTAssertEqual(state.matches.count, 1)
        state.query = "walk"
        state.method = .fullWord
        state.setResults(view.search(for: try XCTUnwrap(state.searchQuery)).map(\.range))
        XCTAssertEqual(state.matches.count, 2)
        XCTAssertEqual(view.text, text)
        XCTAssertEqual(view.selectedRange, selection)
        state.method = .regularExpression
        state.query = "["
        XCTAssertNotNil(state.validationMessage)
        XCTAssertNil(state.searchQuery)
        XCTAssertTrue(state.matches.isEmpty)
        state.move(1)
        XCTAssertEqual(state.countDescription, "0 matches")
        state.query = "walk\\w*"
        state.setResults(view.search(for: try XCTUnwrap(state.searchQuery)).map(\.range))
        XCTAssertEqual(state.matches.count, 3)
        state.isPresented = false
        XCTAssertNil(state.searchQuery)
    }

    @MainActor
    func testFindPreviewsSelectionReplacementAndUndo() throws {
        let state = NoteFindState()
        let view = Runestone.TextView()
        let original = "🌿 café walk\nanother walk"
        view.text = original
        state.isPresented = true
        state.query = "walk"
        state.setResults(view.search(for: try XCTUnwrap(state.searchQuery)).map(\.range), text: original)
        XCTAssertTrue(state.preview(at: 0).contains("café walk"))
        state.select(1)
        XCTAssertEqual(state.countDescription, "2 of 2")
        state.select(9)
        XCTAssertEqual(state.selectedIndex, 1)
        state.replacement = "stroll"
        XCTAssertTrue(state.replace(in: view, all: false))
        XCTAssertEqual(view.text, "🌿 café walk\nanother stroll")
        view.undoManager?.undo()
        XCTAssertEqual(view.text, original)
        view.undoManager?.redo()
        XCTAssertEqual(view.text, "🌿 café walk\nanother stroll")
        view.undoManager?.undo()
        XCTAssertEqual(view.text, original)
        XCTAssertTrue(state.replace(in: view, all: true))
        XCTAssertEqual(view.text, "🌿 café stroll\nanother stroll")
        view.undoManager?.undo()
        XCTAssertEqual(view.text, original)
        view.text = "A different unsaved document"
        XCTAssertFalse(state.replace(in: view, all: true))
        XCTAssertEqual(view.text, "A different unsaved document")
    }

    @MainActor
    func testRegexReplacementCapturesAndReadOnlySafeguards() throws {
        let state = NoteFindState()
        let view = Runestone.TextView()
        view.text = "café-1 café-2"
        state.isPresented = true
        state.method = .regularExpression
        state.query = "café-(\\d)"
        state.replacement = "$1-café"
        state.setResults(view.search(for: try XCTUnwrap(state.searchQuery)).map(\.range), text: view.text)
        view.isEditable = false
        XCTAssertFalse(state.replace(in: view, all: true))
        view.isEditable = true
        XCTAssertTrue(state.replace(in: view, all: true))
        XCTAssertEqual(view.text, "1-café 2-café")
        state.isPresented = false
        XCTAssertFalse(state.replace(in: view, all: true))
    }
}
