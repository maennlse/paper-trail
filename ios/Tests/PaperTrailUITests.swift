// SPDX-License-Identifier: GPL-3.0-or-later
import XCTest

final class PaperTrailUITests: XCTestCase {
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
