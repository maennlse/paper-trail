// SPDX-License-Identifier: GPL-3.0-or-later
import XCTest

final class PaperTrailUITests: XCTestCase {
    func testFolderColorSwatchesShowSelectionAndCreateFolder() {
        let app = XCUIApplication()
        app.launchEnvironment["PAPERTRAIL_TEST_PROFILE"] = UUID().uuidString
        app.launch()
        XCTAssertTrue(app.buttons["folder-toggle"].waitForExistence(timeout: 20))
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"),
                                             object: app.buttons["compose"])
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 20), .completed)
        app.buttons["folder-toggle"].tap()
        app.buttons["New folder"].tap()
        let blue = app.buttons["folder-color-blue"]
        let sage = app.buttons["folder-color-sage"]
        XCTAssertTrue(blue.waitForExistence(timeout: 10))
        for name in ["blue", "sage", "amber", "coral", "violet", "teal"] {
            XCTAssertTrue(app.buttons["folder-color-\(name)"].exists)
        }
        XCTAssertTrue(blue.isSelected)
        sage.tap()
        XCTAssertTrue(sage.isSelected)
        XCTAssertFalse(blue.isSelected)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Folder color swatches with Sage selected"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        let name = app.textFields["new-folder-name"]
        name.tap()
        name.typeText("Sage notes")
        app.buttons["Create"].tap()
        XCTAssertTrue(app.buttons["Sage notes"].waitForExistence(timeout: 10))
        app.buttons["Sage notes"].tap()
        XCTAssertTrue(app.navigationBars["Sage notes"].waitForExistence(timeout: 10))
    }

    func testSyncDefaultsToSingleURLAndUnifiedActions() {
        let app = XCUIApplication()
        app.launchEnvironment["PAPERTRAIL_TEST_PROFILE"] = UUID().uuidString
        app.launch()
        XCTAssertTrue(app.buttons["folder-toggle"].waitForExistence(timeout: 20))
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"),
                                             object: app.buttons["compose"])
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 20), .completed)
        XCTAssertFalse(app.navigationBars.buttons["Sync vault locked or not configured"].exists)
        XCTAssertFalse(app.navigationBars.buttons["Sync vault unlocked"].exists)
        app.buttons["folder-toggle"].tap()
        app.buttons["Sync"].tap()
        app.buttons["Sync Settings"].tap()
        XCTAssertTrue(app.textFields["sync-server"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.textFields["sync-issuer"].exists)
        XCTAssertFalse(app.textFields["sync-client-id"].exists)
        for _ in 0..<4 where !app.buttons["sync-sign-in-out"].exists { app.swipeUp() }
        XCTAssertEqual(app.buttons.matching(identifier: "sync-sign-in-out").count, 1)
        XCTAssertEqual(app.buttons["sync-sign-in-out"].label, "Sign In")
        XCTAssertEqual(app.buttons.matching(identifier: "sync-lock-unlock").count, 1)
        XCTAssertEqual(app.buttons["sync-lock-unlock"].label, "Unlock Vault")
        XCTAssertFalse(app.buttons["Lock Vault"].exists)
        XCTAssertFalse(app.buttons["Advanced Authentication"].exists)
        XCTAssertFalse(app.switches["sync-manual-auth"].exists)
        XCTAssertFalse(app.textFields["sync-scopes"].exists)
    }

    func testRailAndPlainTextRoundTrip() {
        let app = XCUIApplication()
        app.launchEnvironment["PAPERTRAIL_TEST_PROFILE"] = UUID().uuidString
        app.launch()
        XCTAssertTrue(app.buttons["folder-toggle"].waitForExistence(timeout: 20))
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"),
                                             object: app.buttons["compose"])
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 20), .completed)
        app.buttons["folder-toggle"].tap()
        XCTAssertTrue(app.buttons["All Notes"].waitForExistence(timeout: 5))
        app.buttons["All Notes"].tap()
        let railClosed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: app.otherElements["folder-rail"])
        XCTAssertEqual(XCTWaiter.wait(for: [railClosed], timeout: 5), .completed)
        let composeReady = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "hittable == true AND enabled == true"),
            object: app.buttons["compose"])
        XCTAssertEqual(XCTWaiter.wait(for: [composeReady], timeout: 5), .completed)
        app.buttons["compose"].tap()
        let editor = app.scrollViews["note-editor"].textViews.firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        editor.tap()
        editor.typeText("iOS smoke test\nPlain UTF-8 text")
        app.buttons["Done"].tap()
        XCTAssertTrue(app.staticTexts["iOS smoke test"].waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["iOS smoke test"].waitForExistence(timeout: 15))
    }

    func testAllFileTypesRenameUnpinAndSwipeDeletion() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        XCTAssertTrue(app.staticTexts["Reading List"].exists)
        XCTAssertTrue(app.staticTexts["sample.bin"].exists)
        XCTAssertTrue(app.navigationBars.buttons["preferences"].exists)
        app.staticTexts["sample.bin"].tap()
        XCTAssertTrue(app.alerts.firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.scrollViews["note-editor"].exists)
        app.alerts.buttons["OK"].tap()

        app.staticTexts["weekend.md"].press(forDuration: 1)
        app.buttons["Rename Note"].tap()
        let filename = app.textFields["rename-filename"]
        XCTAssertTrue(filename.waitForExistence(timeout: 5))
        filename.tap()
        filename.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: "weekend.md".count))
        filename.typeText("outdoors.md")
        app.buttons["Rename"].tap()
        XCTAssertTrue(app.staticTexts["outdoors.md"].waitForExistence(timeout: 10))
        app.staticTexts["outdoors.md"].tap()
        let editor = app.scrollViews["note-editor"].textViews.firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        XCTAssertTrue((editor.value as? String ?? "").contains("Weekend plans"))
        app.buttons["Done"].tap()

        app.cells.containing(.staticText, identifier: "ideas.txt").firstMatch.swipeRight()
        app.buttons["Unpin"].tap()
        app.staticTexts["ideas.txt"].press(forDuration: 1)
        XCTAssertTrue(app.buttons["Pin Note"].waitForExistence(timeout: 5))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["ideas.txt"].waitForExistence(timeout: 20))
        app.staticTexts["ideas.txt"].press(forDuration: 1)
        XCTAssertTrue(app.buttons["Pin Note"].waitForExistence(timeout: 5))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["Reading List"].waitForExistence(timeout: 20))

        app.cells.containing(.staticText, identifier: "Reading List").firstMatch.swipeLeft()
        app.buttons["Delete"].tap()
        app.buttons["Delete Note"].tap()
        let removed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: app.staticTexts["Reading List"])
        XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 10), .completed)
    }

    func testFolderDragOrderPersists() {
        let app = fixtureApp()
        XCTAssertTrue(app.buttons["folder-toggle"].waitForExistence(timeout: 20))
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.buttons["folder-toggle"].tap()
        let source = app.buttons["Personal"]
        let target = app.buttons["Work Notes"]
        XCTAssertTrue(source.waitForExistence(timeout: 5))
        source.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.5, thenDragTo: target.coordinate(
                withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2)),
                   withVelocity: .slow, thenHoldForDuration: 0.5)
        let reordered = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in source.frame.minY < target.frame.minY }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [reordered], timeout: 10), .completed)
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.buttons["folder-toggle"].tap()
        XCTAssertTrue(source.waitForExistence(timeout: 5))
        XCTAssertLessThan(source.frame.minY, target.frame.minY)
    }

    func testCaptureReadmeScreenshots() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        // Exclude the binary-safety fixture from the demonstration screenshots.
        app.cells.containing(.staticText, identifier: "sample.bin").firstMatch.swipeLeft()
        app.buttons["Delete"].tap()
        app.buttons["Delete Note"].tap()
        let removed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: app.staticTexts["sample.bin"])
        XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 10), .completed)
        app.buttons["folder-toggle"].tap()
        XCTAssertTrue(app.buttons["Personal"].waitForExistence(timeout: 5))
        let library = XCTAttachment(screenshot: app.screenshot())
        library.name = "ios-library"
        library.lifetime = .keepAlways
        add(library)
        app.buttons["All Notes"].tap()
        app.staticTexts["weekend.md"].tap()
        XCTAssertTrue(app.scrollViews["note-editor"].waitForExistence(timeout: 10))
        let editor = XCTAttachment(screenshot: app.screenshot())
        editor.name = "ios-editor"
        editor.lifetime = .keepAlways
        add(editor)
    }

    func testMoveNoteAndEditFolderPersist() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.staticTexts["weekend.md"].press(forDuration: 1)
        app.buttons["Move…"].tap()
        XCTAssertTrue(app.navigationBars["Move Note"].waitForExistence(timeout: 5))
        app.buttons["Work Notes"].tap()
        let row = app.cells.containing(.staticText, identifier: "weekend.md").firstMatch
        let folderBadge = row.descendants(matching: .any).matching(
            NSPredicate(format: "label CONTAINS %@", "Folder: Work Notes")).firstMatch
        XCTAssertTrue(folderBadge.waitForExistence(timeout: 10))
        app.buttons["folder-toggle"].tap()
        app.buttons["Work Notes"].press(forDuration: 1)
        app.buttons["Edit Folder…"].tap()
        let name = app.textFields["new-folder-name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: "Work Notes".count))
        name.typeText("Writing")
        app.buttons["folder-color-coral"].tap()
        app.buttons["Save"].tap()
        let renamedBadge = row.descendants(matching: .any).matching(
            NSPredicate(format: "label CONTAINS %@", "Folder: Writing")).firstMatch
        XCTAssertTrue(renamedBadge.waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.buttons["folder-toggle"].tap()
        XCTAssertTrue(app.buttons["Writing"].waitForExistence(timeout: 5))
        app.buttons["Writing"].press(forDuration: 1)
        app.buttons["Edit Folder…"].tap()
        XCTAssertTrue(app.buttons["folder-color-coral"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["folder-color-coral"].isSelected)
    }

    func testFindAndPrepareNativeShareWithoutChangingNote() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.staticTexts["weekend.md"].tap()
        let editor = app.scrollViews["note-editor"].textViews.firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        let original = editor.value as? String
        XCTAssertTrue(app.staticTexts["Saved locally"].exists)
        app.buttons["note-actions"].tap()
        app.buttons["find-in-note"].tap()
        let search = app.textFields["note-find-query"]
        let findPresented = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == true AND hittable == true"), object: search)
        XCTAssertEqual(XCTWaiter.wait(for: [findPresented], timeout: 20), .completed)
        search.tap()
        search.typeText("walk")
        XCTAssertTrue(app.staticTexts["note-find-count"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["note-find-options"].exists)
        XCTAssertTrue(app.buttons["note-find-next"].exists)
        app.buttons["note-find-expand"].tap()
        XCTAssertTrue(app.buttons["note-find-result-0"].waitForExistence(timeout: 5))
        app.buttons["note-find-result-0"].tap()
        let findScreenshot = XCTAttachment(screenshot: app.screenshot())
        findScreenshot.name = "ios-find-expanded"
        findScreenshot.lifetime = .keepAlways
        add(findScreenshot)
        app.buttons["note-find-expand"].tap()
        XCTAssertEqual(editor.value as? String, original)
        app.buttons["note-find-close"].tap()
        app.navigationBars.buttons["Done"].tap()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 10))
        app.staticTexts["weekend.md"].tap()
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        app.buttons["note-actions"].tap()
        app.buttons["share-note"].tap()
        XCTAssertTrue(app.navigationBars["Share Note"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["weekend.md"].exists)
        XCTAssertTrue(app.buttons["share-export"].exists)
        XCTAssertTrue(app.buttons["share-text"].exists)
        XCTAssertTrue(app.staticTexts.matching(
            NSPredicate(format: "label BEGINSWITH %@", "This copy is not encrypted.")).firstMatch.exists)
        app.navigationBars["Share Note"].buttons["Done"].tap()
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        XCTAssertEqual(editor.value as? String, original)
        XCTAssertTrue(app.staticTexts["Saved locally"].exists)
    }

    func testEditorPreferencesPersistAndKeepNoteContents() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.buttons["preferences"].tap()
        let numbers = app.switches["editor-line-numbers"]
        let wrap = app.switches["editor-wrap-text"]
        let highlight = app.switches["editor-highlight-current-line"]
        XCTAssertTrue(numbers.waitForExistence(timeout: 5))
        XCTAssertEqual(numbers.value as? String, "1")
        XCTAssertEqual(wrap.value as? String, "1")
        XCTAssertEqual(highlight.value as? String, "1")
        setSwitch(numbers, enabled: false)
        setSwitch(wrap, enabled: false)
        setSwitch(highlight, enabled: false)
        app.buttons["editor-font-choice"].tap()
        app.buttons["Monospaced"].tap()
        XCTAssertTrue(app.staticTexts["editor-font-preview"].exists)
        app.navigationBars["Preferences"].buttons["Done"].tap()
        let saved = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: app.navigationBars["Preferences"])
        XCTAssertEqual(XCTWaiter.wait(for: [saved], timeout: 10), .completed)
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.buttons["preferences"].tap()
        XCTAssertTrue(numbers.waitForExistence(timeout: 5))
        XCTAssertEqual(numbers.value as? String, "0")
        XCTAssertEqual(wrap.value as? String, "0")
        XCTAssertEqual(highlight.value as? String, "0")
        let fontChoice = app.buttons["editor-font-choice"]
        XCTAssertTrue(fontChoice.label.contains("Monospaced") ||
                      (fontChoice.value as? String)?.contains("Monospaced") == true)
        fontChoice.tap()
        app.buttons["Custom Font"].tap()
        let cancelFont = app.buttons.matching(
            NSPredicate(format: "label IN %@", ["Cancel", "Close", "close"])).firstMatch
        XCTAssertTrue(cancelFont.waitForExistence(timeout: 10))
        cancelFont.tap()
        XCTAssertTrue(fontChoice.waitForExistence(timeout: 5))
        let cancelled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            fontChoice.label.contains("Monospaced") ||
                (fontChoice.value as? String)?.contains("Monospaced") == true
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [cancelled], timeout: 5), .completed)
        fontChoice.tap()
        app.buttons["System Default"].tap()
        setSwitch(numbers, enabled: true)
        setSwitch(wrap, enabled: true)
        setSwitch(highlight, enabled: true)
        app.navigationBars["Preferences"].buttons["Done"].tap()
        let dismissed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: app.navigationBars["Preferences"])
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 10), .completed)
        app.staticTexts["weekend.md"].tap()
        let editor = app.scrollViews["note-editor"].textViews.firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        XCTAssertEqual(editor.value as? String,
                       "Weekend plans\nA long walk, a good book, and time to write.")
    }

    func testSyntaxChoiceHeaderAndSwipeBackPreserveNote() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.staticTexts["weekend.md"].tap()
        let editor = app.scrollViews["note-editor"].textViews.firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        let original = editor.value as? String
        XCTAssertEqual(app.staticTexts["editor-note-title"].label, "Weekend plans")
        XCTAssertEqual(app.staticTexts["editor-note-filename"].label, "weekend.md")
        XCTAssertTrue(app.buttons["note-language"].label.contains("Markdown"))
        app.buttons["note-language"].tap()
        app.buttons["note-language-plain"].tap()
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["note-language"].label.contains("Plain Text"))
        XCTAssertEqual(editor.value as? String, original)
        editor.tap()
        editor.typeText("\nSaved by edge swipe")
        let edited = editor.value as? String
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.01, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo:
                app.coordinate(withNormalizedOffset: CGVector(dx: 0.7, dy: 0.5)))
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.staticTexts["weekend.md"].tap()
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["note-language"].label.contains("Plain Text"))
        app.buttons["note-language"].tap()
        app.buttons["note-language-automatic"].tap()
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["note-language"].label.contains("Markdown"))
        XCTAssertEqual(editor.value as? String, edited)
    }

    func testBottomSearchAndPlusCompose() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        XCTAssertTrue(app.buttons["compose"].isHittable)
        XCTAssertEqual(app.toolbars.buttons.matching(identifier: "compose").count, 0)
        let search = librarySearch(in: app)
        let visible = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == true AND hittable == true"), object: search)
        XCTAssertEqual(XCTWaiter.wait(for: [visible], timeout: 5), .completed)
        XCTAssertGreaterThan(search.frame.midY, app.frame.midY)
        search.tap()
        search.typeText("long walk")
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 5))
        let filtered = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: app.staticTexts["ideas.txt"])
        XCTAssertEqual(XCTWaiter.wait(for: [filtered], timeout: 5), .completed)
        app.staticTexts["weekend.md"].tap()
        XCTAssertTrue(app.scrollViews["note-editor"].waitForExistence(timeout: 10))
        app.navigationBars.buttons["Done"].tap()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 10))
        XCTAssertEqual(search.value as? String, "long walk")
        XCTAssertFalse(app.staticTexts["ideas.txt"].exists)
    }

    func testEmptyLibraryKeepsBottomSearchVisibleAfterPullToRefresh() {
        let app = XCUIApplication()
        app.launchEnvironment["PAPERTRAIL_TEST_PROFILE"] = UUID().uuidString
        app.launch()
        XCTAssertTrue(app.buttons["compose"].waitForExistence(timeout: 20))
        let ready = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "enabled == true"), object: app.buttons["compose"])
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 20), .completed)
        let search = librarySearch(in: app)
        XCTAssertTrue(search.isHittable)
        XCTAssertGreaterThan(search.frame.midY, app.frame.midY)
        app.collectionViews.firstMatch.swipeDown()
        let revealed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == true AND hittable == true"), object: search)
        XCTAssertEqual(XCTWaiter.wait(for: [revealed], timeout: 5), .completed)
    }

    func testFullSwipeDeleteStillRequiresConfirmationAndCancelKeepsTheNote() {
        let app = fixtureApp()
        let note = app.staticTexts["weekend.md"]
        XCTAssertTrue(note.waitForExistence(timeout: 20))
        let y = note.frame.midY / app.frame.height
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: y))
        start.press(forDuration: 0.1, thenDragTo: app.coordinate(
            withNormalizedOffset: CGVector(dx: 0.02, dy: y)))
        XCTAssertTrue(app.buttons["Delete Note"].waitForExistence(timeout: 5))
        app.buttons["Cancel"].tap()
        XCTAssertTrue(note.waitForExistence(timeout: 5))
    }

    func testAboutShowsVersionAndSourceLink() {
        let app = fixtureApp()
        XCTAssertTrue(app.staticTexts["weekend.md"].waitForExistence(timeout: 20))
        app.buttons["preferences"].tap()
        let version = app.descendants(matching: .any).matching(identifier: "about-version").firstMatch
        let source = app.descendants(matching: .any).matching(identifier: "about-source").firstMatch
        for _ in 0..<8 {
            if source.exists && source.isHittable { break }
            app.swipeUp()
        }
        XCTAssertTrue(version.exists)
        XCTAssertTrue(source.exists)
        XCTAssertTrue(source.isHittable)
        XCTAssertFalse(version.label.contains("Unknown"))
    }

    private func librarySearch(in app: XCUIApplication) -> XCUIElement {
        app.textFields["library-search"]
    }

    private func setSwitch(_ control: XCUIElement, enabled: Bool,
                           file: StaticString = #filePath, line: UInt = #line) {
        let expected = enabled ? "1" : "0"
        if control.value as? String != expected {
            // SwiftUI exposes the whole form row as a switch. Its center can
            // be empty space; target the actual trailing UISwitch instead.
            control.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
        }
        let changed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value == %@", expected), object: control)
        XCTAssertEqual(XCTWaiter.wait(for: [changed], timeout: 5), .completed, file: file, line: line)
    }

    private func fixtureApp() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["PAPERTRAIL_TEST_PROFILE"] = UUID().uuidString
        app.launchEnvironment["PAPERTRAIL_TEST_FIXTURES"] = "1"
        app.launch()
        return app
    }
}
