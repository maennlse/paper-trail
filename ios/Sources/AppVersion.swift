// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

enum AppVersion {
    static let sourceURL = URL(string: "https://github.com/maennlse/paper-trail")!

    static func description(info: [String: Any]) -> String {
        let version = (info["CFBundleShortVersionString"] as? String)
            .flatMap { $0.isEmpty ? nil : $0 } ?? "Unknown"
        guard let build = info["CFBundleVersion"] as? String, !build.isEmpty else { return version }
        return "\(version) (\(build))"
    }
}
