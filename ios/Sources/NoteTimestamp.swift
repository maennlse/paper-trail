// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

enum NoteTimestamp {
    /// Match GTK: today's time, month/day this year, otherwise an ISO short date.
    static func description(_ date: Date, now: Date = Date(),
                            calendar: Calendar = .current, locale: Locale = .current) -> String {
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = locale
        if calendar.isDate(date, inSameDayAs: now) {
            formatter.dateFormat = "HH:mm"
        } else if calendar.component(.year, from: date) == calendar.component(.year, from: now) {
            formatter.dateFormat = "MMM dd"
        } else {
            formatter.dateFormat = "yyyy-MM-dd"
        }
        return formatter.string(from: date)
    }
}
