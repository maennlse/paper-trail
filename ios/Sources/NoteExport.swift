// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

/// Own only this export's temporary directory; never share the live note URL.
final class NoteExport: Identifiable {
    let id: UUID
    let url: URL
    private let directory: URL

    init(filename: String, data: Data) throws {
        let id = UUID()
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("papertrail-export-\(id.uuidString)", isDirectory: true)
        let name = URL(fileURLWithPath: filename).lastPathComponent
        guard !name.isEmpty, name != ".", name != "..", name != "/" else {
            throw CocoaError(.fileWriteInvalidFileName)
        }
        let url = directory.appendingPathComponent(name)
        self.id = id
        self.directory = directory
        self.url = url
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        do { try data.write(to: url, options: .atomic) }
        catch {
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
    }

    deinit { try? FileManager.default.removeItem(at: directory) }
}
