// SPDX-License-Identifier: GPL-3.0-or-later
import Runestone
import UIKit
import XCTest

final class SyntaxHighlightingTests: XCTestCase {
    @MainActor
    func testBundledGrammarsParseWithoutChangingTextOrSelection() async {
        let highlighter = SyntaxHighlighter()
        let samples = [
            "markdown": "# Heading\n**bold** and `code`\n",
            "json": "{\"answer\": 42}",
            "python3": "def answer():\n    return 42\n",
            "rust": "fn main() { let answer = 42; }\n",
            "js": "const answer = \"hello\";\n",
            "swift": "let answer = 42\n",
            "toml": "[note]\nanswer = 42\n",
            "yaml": "answer: true\n",
        ]
        for (id, text) in samples {
            let language = highlighter.treeSitterLanguage(named: id)
            XCTAssertNotNil(language?.highlightsQuery, id)
            XCTAssertNil(language?.indentationScopes, "Highlighting must not enable autoindent")
            let view = Runestone.TextView()
            view.text = text
            view.selectedRange = NSRange(location: 1, length: 2)
            let parsed = expectation(description: "Parse \(id)")
            view.setLanguageMode(highlighter.mode(for: id)) { success in
                XCTAssertTrue(success, id)
                parsed.fulfill()
            }
            await fulfillment(of: [parsed], timeout: 10)
            XCTAssertNotNil(view.syntaxNode(at: 1), id)
            XCTAssertEqual(view.text, text)
            XCTAssertEqual(view.selectedRange, NSRange(location: 1, length: 2))
        }
        XCTAssertNotNil(highlighter.treeSitterLanguage(named: "markdown_inline")?.highlightsQuery)
    }

    @MainActor
    func testPlainTextAndUnknownLanguagesDoNotCreateParsers() {
        let highlighter = SyntaxHighlighter()
        XCTAssertTrue(highlighter.mode(for: "plain") is PlainTextLanguageMode)
        XCTAssertTrue(highlighter.mode(for: "not-bundled") is PlainTextLanguageMode)
        XCTAssertNil(highlighter.treeSitterLanguage(named: "not-bundled"))
    }
}
