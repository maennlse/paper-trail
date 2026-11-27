// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation
import Runestone
import TreeSitterMarkdownRunestone
import TreeSitterMarkdownInlineRunestone
import TreeSitterJSONRunestone
import TreeSitterPythonRunestone
import TreeSitterRustRunestone
import TreeSitterJavaScriptRunestone
import TreeSitterSwiftRunestone
import TreeSitterTOMLRunestone
import TreeSitterYAMLRunestone

/// The parser set is pinned to the Tree-sitter ABI used by Runestone 0.5.2.
/// Unknown/injected languages remain readable plain text.
final class SyntaxHighlighter: TreeSitterLanguageProvider {
    private var cache: [String: TreeSitterLanguage] = [:]
    private let lock = NSLock()

    func mode(for id: String) -> LanguageMode {
        guard let language = treeSitterLanguage(named: id) else { return PlainTextLanguageMode() }
        return TreeSitterLanguageMode(language: language, languageProvider: self)
    }

    func treeSitterLanguage(named name: String) -> TreeSitterLanguage? {
        // Embedded-language requests can arrive from Runestone's parser queue.
        lock.lock()
        defer { lock.unlock() }
        let key = name.lowercased()
        if let language = cache[key] { return language }
        let original: TreeSitterLanguage
        switch key {
        case "markdown", "md": original = .markdown
        case "markdown_inline": original = .markdownInline
        case "json": original = .json
        case "python", "python3", "py": original = .python
        case "rust", "rs": original = .rust
        case "javascript", "js", "jsx": original = .javaScript
        case "swift": original = .swift
        case "toml": original = .toml
        case "yaml", "yml": original = .yaml
        default: return nil
        }
        // Syntax highlighting is visual only: do not introduce grammar-driven
        // indentation, bracket insertion, or other code-editing behavior.
        let language = TreeSitterLanguage(original.languagePointer,
                                          highlightsQuery: original.highlightsQuery,
                                          injectionsQuery: original.injectionsQuery)
        language.prepare()
        cache[key] = language
        return language
    }
}
