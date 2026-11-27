// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

enum SyncRelativeTime {
    static func description(since date: Date, now: Date) -> String {
        let seconds = max(0, now.timeIntervalSince(date))
        if seconds < 60 { return "Last synced: Just now" }
        let formatter = DateComponentsFormatter()
        formatter.allowedUnits = seconds < 3600 ? [.minute] :
            seconds < 86400 ? [.hour] : [.day]
        formatter.unitsStyle = .full
        formatter.maximumUnitCount = 1
        let rounded = Date(timeInterval: floor(seconds / 60) * 60, since: date)
        let elapsed = formatter.string(from: date, to: rounded) ?? "1 minute"
        return "Last synced: \(elapsed) ago"
    }
}
