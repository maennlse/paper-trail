// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

/// Glass belongs to the control layer, never the note's reading/editing surface.
private struct GlassSurface: ViewModifier {
    let radius: CGFloat
    let interactive: Bool
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    @ViewBuilder
    func body(content: Content) -> some View {
        if reduceTransparency || contrast == .increased {
            content.background(Color(uiColor: .secondarySystemBackground),
                               in: RoundedRectangle(cornerRadius: radius))
                .overlay {
                    RoundedRectangle(cornerRadius: radius).stroke(.secondary, lineWidth: 1)
                        .allowsHitTesting(false)
                }
        } else {
            modernSurface(content)
        }
    }

    @ViewBuilder
    private func modernSurface(_ content: Content) -> some View {
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            content.glassEffect(interactive ? .regular.interactive() : .regular,
                                in: RoundedRectangle(cornerRadius: radius))
        } else {
            fallback(content)
        }
        #else
        fallback(content)
        #endif
    }

    private func fallback(_ content: Content) -> some View {
        content.background(.regularMaterial, in: RoundedRectangle(cornerRadius: radius))
            .overlay {
                RoundedRectangle(cornerRadius: radius).stroke(.quaternary)
                    .allowsHitTesting(false)
            }
            .shadow(color: .black.opacity(0.08), radius: 10, y: 3)
    }
}

extension View {
    func controlGlass(radius: CGFloat = 28, interactive: Bool = false) -> some View {
        modifier(GlassSurface(radius: radius, interactive: interactive))
    }

    /// Let the OS supply sheet materials, dimming, and toolbar glass.
    func nativeSheet() -> some View {
        presentationDragIndicator(.visible)
            .presentationCornerRadius(32)
    }
}
