// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

/// One folder silhouette/initials treatment for the sidebar and note overview.
struct FolderBadge: View {
    let badge: String
    let color: Color
    var size: CGFloat = 38

    var body: some View {
        ZStack {
            Image(systemName: "folder.fill")
                .font(.system(size: size)).foregroundStyle(color)
            Text(badge)
                .font(.system(size: size * 10 / 38, weight: .bold))
                .foregroundStyle(.white).lineLimit(1).minimumScaleFactor(0.7)
                .frame(width: size * 30 / 38).offset(y: size * 3 / 38)
        }
        .frame(width: size * 48 / 38, height: size * 48 / 38)
    }
}
