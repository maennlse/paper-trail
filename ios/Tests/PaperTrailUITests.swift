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
        XCTAssertTrue(app.textFields["sync-server"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.textFields["OIDC issuer URL (HTTPS)"].exists)
        XCTAssertFalse(app.textFields["Public client ID"].exists)
        for _ in 0..<4 where !app.buttons["sync-sign-in-out"].exists { app.swipeUp() }
        XCTAssertEqual(app.buttons.matching(identifier: "sync-sign-in-out").count, 1)
        XCTAssertEqual(app.buttons["sync-sign-in-out"].label, "Sign In")
        XCTAssertEqual(app.buttons.matching(identifier: "sync-lock-unlock").count, 1)
        XCTAssertEqual(app.buttons["sync-lock-unlock"].label, "Unlock Vault")
        XCTAssertFalse(app.buttons["Lock Vault"].exists)
        for _ in 0..<4 where !app.buttons["Advanced Authentication"].isHittable { app.swipeDown() }
        app.buttons["Advanced Authentication"].tap()
        app.switches["sync-manual-auth"].tap()
        XCTAssertTrue(app.textFields["OIDC issuer URL (HTTPS)"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.textFields["Public client ID"].exists)
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
        app.buttons["compose"].tap()
        let editor = app.textViews["note-editor"]
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        editor.tap()
        editor.typeText("iOS smoke test\nPlain UTF-8 text")
        app.buttons["Done"].tap()
        XCTAssertTrue(app.staticTexts["iOS smoke test"].waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["iOS smoke test"].waitForExistence(timeout: 15))
    }
}
