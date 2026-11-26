// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct SyncSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var server = ""
    @State private var issuer = ""
    @State private var clientID = ""
    @State private var scopes = "openid profile offline_access"
    @State private var manualAuthentication = false
    @State private var folders = Set<String>()
    @State private var remoteFolders: [SyncFolder] = []
    @State private var setup: SyncSetup?
    @State private var confirmTrust = false
    @State private var draftLoaded = false
    @State private var savedDraft: SyncProfile?

    private var profile: SyncProfile {
        SyncProfile(server: server, issuer: issuer, clientId: clientID,
                    scopes: scopes, folders: folders.sorted(), discoverAuth: !manualAuthentication)
    }

    var body: some View {
        NavigationStack {
            Form {
                statusSection
                connectionSection
                vaultSection
                if let recovery = model.recovery { recoverySection(recovery) }
                remoteFolderSection
                Section {
                    Text("Sync is end-to-end encrypted. Local .txt files are not encrypted by the app. Sync runs while the app is open.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Encrypted Sync")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .disabled(model.recovery != nil)
                }
            }
            .task { await load() }
            .onChange(of: profile) { _, _ in invalidateSetup() }
            .onDisappear { invalidateSetup() }
            .confirmationDialog("Trust Identity Provider?", isPresented: $confirmTrust, titleVisibility: .visible) {
                Button("Trust and Save") {
                    if let token = setup?.token {
                        Task { await save(token) }
                    }
                }
                Button("Cancel", role: .cancel) { invalidateSetup() }
            } message: {
                Text(setup?.trustDescription ?? "")
            }
            .sheet(item: $model.vaultPrompt) { prompt in
                VaultPassphraseSheet(prompt: prompt)
            }
            .alert("Paper Trail", isPresented: Binding(
                get: { model.errorMessage != nil },
                set: { if !$0 { model.errorMessage = nil } }
            )) {
                Button("OK") { model.errorMessage = nil }
            } message: { Text(model.errorMessage ?? "") }
        }
        .interactiveDismissDisabled(model.recovery != nil)
    }

    private var statusSection: some View {
        Section("Status") {
            Label(model.syncMessage, systemImage: model.unlocked ? "lock.open" : "lock.fill")
            if model.busy {
                ProgressView()
                Button("Cancel Operation") { model.cancelOperation() }
            }
            Button("Refresh Status") {
                Task { await model.sync(.status); await loadFolders() }
            }.disabled(model.busy || !model.configured)
            Button("Sync Now") {
                Task { await model.sync(.synchronize); await loadFolders() }
            }.disabled(model.busy || !model.unlocked || model.recovery != nil)
        }
    }

    private var connectionSection: some View {
        Section {
            TextField("Server URL (HTTPS)", text: $server)
                .keyboardType(.URL).accessibilityIdentifier("sync-server")
            Text(manualAuthentication
                 ? "Manual authentication is enabled under Advanced Authentication."
                 : "Enter your sync server URL. Authentication settings are discovered automatically; you will be asked before trusting a new identity provider.")
                .font(.caption).foregroundStyle(.secondary)
            DisclosureGroup("Advanced Authentication") {
                Toggle("Configure authentication manually", isOn: $manualAuthentication)
                    .accessibilityIdentifier("sync-manual-auth")
                if manualAuthentication {
                    TextField("OIDC issuer URL (HTTPS)", text: $issuer).keyboardType(.URL)
                    TextField("Public client ID", text: $clientID)
                    TextField("Scopes", text: $scopes)
                }
                Text("The identity provider must register io.github.maennlse.paper-trail://oauth/callback for the public client. No client secret is used.")
                    .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            }
            .accessibilityIdentifier("sync-advanced")
            ForEach(model.folders, id: \.path) { folder in
                Toggle(folder.name, isOn: Binding(
                    get: { folders.contains(folder.path) },
                    set: { if $0 { folders.insert(folder.path) } else { folders.remove(folder.path) } }
                ))
            }
            Button("Save Sync Configuration") { Task { await configure() } }
                .disabled(!draftLoaded || server.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } header: { Text("Connection & Upload Folders") }
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .disabled(model.busy || model.recovery != nil)
    }

    private var vaultSection: some View {
        Section("Account & Encrypted Vault") {
            Button(model.signedIn ? "Sign Out" : "Sign In") {
                if model.signedIn {
                    Task { await model.sync(.logout) }
                } else {
                    Task { await model.signIn() }
                }
            }
            .disabled(!model.signedIn && (model.busy || !model.configured || savedDraft != profile))
            .accessibilityIdentifier("sync-sign-in-out")

            Button(model.unlocked ? "Lock Vault" : "Unlock Vault") {
                if model.unlocked {
                    Task { await model.sync(.lock) }
                } else {
                    model.vaultPrompt = .unlock
                }
            }
            .disabled(!model.unlocked && (model.busy || !model.signedIn || model.recovery != nil))
            .accessibilityIdentifier("sync-lock-unlock")

            if model.vaultExists != true {
                Button("Create Encrypted Vault") { model.vaultPrompt = .create }
                    .disabled(model.busy || !model.signedIn || model.recovery != nil)
            }
            Button("Change Vault Passphrase") { model.vaultPrompt = .change }
                .disabled(model.busy || !model.signedIn || !model.unlocked || model.recovery != nil)
        }
    }

    private func recoverySection(_ recovery: String) -> some View {
        Section("Save Your Recovery Key") {
            Text(recovery).font(.system(.body, design: .monospaced))
                .textSelection(.enabled).privacySensitive()
            Text("Store this key somewhere safe. It is not written to settings or logs. Losing the passphrase, this key, and all unlocked devices means your notes cannot be recovered.")
            Button("I Have Saved the Recovery Key") {
                Task { await model.sync(.confirmRecovery) }
            }.disabled(model.busy)
            Button("Cancel and Lock Vault", role: .cancel) {
                Task { await model.sync(.lock) }
            }
        }
    }

    @ViewBuilder private var remoteFolderSection: some View {
        if !remoteFolders.isEmpty {
            Section("Remote Folders") {
                ForEach(remoteFolders, id: \.id) { folder in
                    HStack {
                        Text(folder.name)
                        Spacer()
                        if folder.selected {
                            Image(systemName: "checkmark")
                        } else if let id = folder.id {
                            Button("Download") {
                                Task {
                                    do {
                                        try await model.call { try $0.acceptSyncFolder(id: id) }
                                        await loadFolders()
                                    } catch { model.errorMessage = error.localizedDescription }
                                }
                            }.disabled(model.busy)
                        }
                    }
                }
            }
        }
    }

    private func invalidateSetup() {
        guard let token = setup?.token else { return }
        setup = nil
        confirmTrust = false
        Task { try? await model.call { try $0.discardSyncSetup(token: token) } }
    }

    private func load() async {
        do {
            let saved = try await model.call { try $0.syncProfile() }
            server = saved.server
            issuer = saved.issuer
            clientID = saved.clientId
            if !saved.scopes.isEmpty { scopes = saved.scopes }
            manualAuthentication = !saved.server.isEmpty && !saved.discoverAuth
            folders = Set(saved.folders)
            model.updateSyncStatus()
            savedDraft = profile
            draftLoaded = true
            await loadFolders()
        } catch { model.errorMessage = error.localizedDescription }
    }

    private func loadFolders() async {
        do {
            remoteFolders = try await model.call { try $0.syncFolders() }.filter { $0.id != nil }
        } catch { model.errorMessage = error.localizedDescription }
    }

    private func configure() async {
        invalidateSetup()
        let submitted = profile
        guard let candidate = await model.prepareConfiguration(submitted) else { return }
        guard submitted == profile else {
            try? await model.call { try $0.discardSyncSetup(token: candidate.token) }
            return
        }
        setup = candidate
        if candidate.needsConfirmation {
            confirmTrust = true
        } else {
            await save(candidate.token)
        }
    }

    private func save(_ token: UInt64) async {
        if await model.confirmConfiguration(token) {
            setup = nil
            savedDraft = profile
            await loadFolders()
        }
    }
}
