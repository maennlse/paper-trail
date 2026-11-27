// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

enum VaultPrompt: String, Identifiable {
    case create, unlock, recover, change
    var id: String { rawValue }
    var title: String {
        switch self {
        case .create: return "Create Encrypted Vault"
        case .unlock: return "Unlock Vault"
        case .recover: return "Recover Vault"
        case .change: return "Change Vault Passphrase"
        }
    }
    var action: SyncAction {
        switch self {
        case .create: return .create
        case .unlock: return .unlock
        case .recover: return .recover
        case .change: return .changePassphrase
        }
    }
    var needsConfirmation: Bool { self == .create || self == .change }
}

struct VaultPassphraseSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    let prompt: VaultPrompt
    @State private var mode: VaultPrompt
    @State private var secret = ""
    @State private var confirmation = ""
    @State private var remember = false
    @State private var validationError: String?

    init(prompt: VaultPrompt) {
        self.prompt = prompt
        _mode = State(initialValue: prompt)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    SecureField(mode == .recover ? "Recovery key" : "Vault passphrase", text: $secret)
                        .accessibilityIdentifier("vault-secret")
                    if mode.needsConfirmation {
                        SecureField("Confirm passphrase", text: $confirmation)
                            .accessibilityIdentifier("vault-confirmation")
                    }
                } footer: {
                    Text(mode == .recover
                         ? "Your recovery key unlocks the existing vault; it does not reset sign-in. Recovery keys are never saved."
                         : "Encryption is separate from sign-in. Local note files remain plaintext. New passphrases need at least 16 characters.")
                }
                if mode != .recover {
                    Section {
                        Toggle("Remember encryption password on this device", isOn: $remember)
                    } footer: {
                        Text("Automatically unlock and sync at startup. Anyone with access to this device's unlocked Keychain can unlock sync.")
                    }
                }
                if mode == .unlock || mode == .recover {
                    Button(mode == .unlock ? "Use Recovery Key Instead" : "Use Passphrase Instead") {
                        secret = ""
                        confirmation = ""
                        remember = false
                        mode = mode == .unlock ? .recover : .unlock
                    }
                }
            }
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .disabled(model.busy)
            .navigationTitle(mode.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { clear(); dismiss() }.disabled(model.busy)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Continue") { submit() }.disabled(secret.isEmpty || model.busy)
                }
            }
            .onDisappear { clear() }
            .alert("Paper Trail", isPresented: Binding(
                get: { validationError != nil || model.errorMessage != nil },
                set: { if !$0 { validationError = nil; model.errorMessage = nil } }
            )) {
                Button("OK") { validationError = nil; model.errorMessage = nil }
            } message: { Text(validationError ?? model.errorMessage ?? "") }
        }
        .interactiveDismissDisabled(model.busy)
    }

    private func clear() {
        secret = ""
        confirmation = ""
        remember = false
    }

    private func submit() {
        guard !mode.needsConfirmation || secret == confirmation else {
            clear()
            validationError = "Passphrases do not match."
            return
        }
        let value = secret
        let shouldRemember = mode != .recover && remember
        let action = mode.action
        clear()
        Task {
            if await model.sync(action, secret: value, remember: shouldRemember) {
                dismiss()
            }
        }
    }
}
