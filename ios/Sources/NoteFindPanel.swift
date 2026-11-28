// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct NoteFindPanel: View {
    @ObservedObject var state: NoteFindState
    @FocusState private var focused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ScaledMetric private var resultsHeight: CGFloat = 180

    var body: some View {
        VStack(spacing: 8) {
            Button { state.expanded.toggle() } label: {
                Capsule().fill(.tertiary).frame(width: 36, height: 5)
                    .frame(maxWidth: .infinity).frame(height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(state.expanded ? "Hide matches" : "Show matches")
            .accessibilityIdentifier("note-find-expand")
            .accessibilityValue(state.expanded ? "Expanded" : "Collapsed")
            .gesture(DragGesture(minimumDistance: 15).onEnded {
                if $0.translation.height < -20 { state.expanded = true }
                if $0.translation.height > 20 { state.expanded = false }
            })
            if state.expanded {
                results
                Divider()
            }
            HStack(spacing: 8) {
                HStack {
                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                    TextField("Find", text: $state.query)
                        .focused($focused).submitLabel(.search)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .onSubmit { state.move(1) }
                        .accessibilityIdentifier("note-find-query")
                }
                .padding(12)
                .background(.background.opacity(0.7), in: RoundedRectangle(cornerRadius: 16))
                Menu {
                    Toggle("Replace", isOn: $state.showsReplacement)
                    Divider()
                    Toggle("Case Sensitive", isOn: $state.caseSensitive)
                    Divider()
                    Picker("Match", selection: $state.method) {
                        ForEach(NoteFindState.Method.allCases) { method in
                            Text(method.rawValue).tag(method)
                        }
                    }
                    .pickerStyle(.inline)
                } label: {
                    Image(systemName: "ellipsis.circle").font(.title2).frame(width: 44, height: 44)
                }
                .accessibilityLabel("Find Options").accessibilityIdentifier("note-find-options")
            }
            if state.showsReplacement {
                TextField("Replace with", text: $state.replacement)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                    .padding(12)
                    .background(.background.opacity(0.7), in: RoundedRectangle(cornerRadius: 16))
                    .accessibilityIdentifier("note-find-replacement")
                HStack {
                    Button("Replace") { state.replaceAction?(false) }
                        .accessibilityIdentifier("note-find-replace")
                    Spacer()
                    Button("Replace All") { state.replaceAction?(true) }
                        .accessibilityIdentifier("note-find-replace-all")
                }
                .frame(minHeight: 44)
                .disabled(state.matches.isEmpty || state.validationMessage != nil)
            }
            HStack(spacing: 0) {
                Button(action: close) { Image(systemName: "xmark").frame(width: 44, height: 44) }
                    .accessibilityLabel("Close Find").accessibilityIdentifier("note-find-close")
                Spacer(minLength: 0)
                Text(state.validationMessage ?? state.countDescription)
                    .font(.caption).foregroundStyle(state.validationMessage == nil ? Color.secondary : .orange)
                    .accessibilityIdentifier("note-find-count")
                Spacer(minLength: 0)
                Button { state.move(-1) } label: {
                    Image(systemName: "chevron.up").frame(width: 44, height: 44)
                }
                .accessibilityLabel("Previous Match").accessibilityIdentifier("note-find-previous")
                .disabled(state.matches.isEmpty)
                Button { state.move(1) } label: {
                    Image(systemName: "chevron.down").frame(width: 44, height: 44)
                }
                .accessibilityLabel("Next Match").accessibilityIdentifier("note-find-next")
                .disabled(state.matches.isEmpty)
            }
        }
        .padding(.horizontal, 12).padding(.bottom, 8).foregroundStyle(.primary)
        .controlGlass(radius: 28)
        .frame(maxWidth: 620)
        .padding(.horizontal, 16).padding(.bottom, 8)
        .animation(reduceMotion ? nil : .snappy(duration: 0.22), value: state.expanded)
        .task { focused = true }
        .accessibilityAction(.escape) { close() }
    }

    private var results: some View {
        ScrollViewReader { proxy in
            ScrollView {
                if state.matches.isEmpty {
                    Text(state.validationMessage ?? "No matches")
                        .foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 100)
                } else {
                    LazyVStack(spacing: 4) {
                        ForEach(state.matches.indices, id: \.self) { index in
                            Button { state.select(index) } label: {
                                HStack(alignment: .top, spacing: 12) {
                                    Text("\(index + 1)").font(.caption.monospacedDigit())
                                        .foregroundStyle(.secondary)
                                    Text(state.preview(at: index)).lineLimit(2)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                    if index == state.selectedIndex {
                                        Image(systemName: "checkmark").foregroundStyle(Color.accentColor)
                                    }
                                }
                                .padding(10).frame(minHeight: 44)
                                .background(index == state.selectedIndex ? Color.accentColor.opacity(0.12) : .clear,
                                            in: RoundedRectangle(cornerRadius: 12))
                            }
                            .buttonStyle(.plain).id(index)
                            .accessibilityLabel("Match \(index + 1): \(state.preview(at: index))")
                            .accessibilityAddTraits(index == state.selectedIndex ? [.isSelected] : [])
                            .accessibilityIdentifier("note-find-result-\(index)")
                        }
                    }
                }
            }
            .frame(height: min(resultsHeight, 220))
            .onChange(of: state.selectedIndex) { _, index in
                proxy.scrollTo(index, anchor: .center)
            }
        }
        .accessibilityIdentifier("note-find-results")
    }

    private func close() {
        focused = false
        state.isPresented = false
    }
}
