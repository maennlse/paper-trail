// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct SyncSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var server = ""
    @State private var issuer = ""
    @State private var clientID = ""
    @State private var scopes = "openid profile offline_access"
    @State private var folders = Set<String>()
    @State private var remoteFolders: [SyncFolder] = []
    @State private var secret = ""
    @State private var confirmCreate = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Status") {
                    Label(model.syncMessage, systemImage: model.unlocked ? "lock.open" : "lock.fill")
                    if model.busy { ProgressView() }
                    Button("Refresh Status") { Task { await model.sync(.status); await loadFolders() } }
                }
                Section {
                    TextField("Server URL (HTTPS)", text: $server).keyboardType(.URL)
                    TextField("OIDC issuer URL (HTTPS)", text: $issuer).keyboardType(.URL)
                    TextField("Public client ID", text: $clientID)
                    TextField("Scopes", text: $scopes)
                    Text("The issuer must register io.github.maennlse.paper-trail://oauth/callback for this public client. No client secret is used.")
                        .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                    ForEach(model.folders, id: \.path) { folder in
                        Toggle(folder.name, isOn: Binding(
                            get: { folders.contains(folder.path) },
                            set: { if $0 { folders.insert(folder.path) } else { folders.remove(folder.path) } }
                        ))
                    }
                    Button("Save Sync Configuration") { Task { await configure() } }
                    Button("Sign In with Browser") { Task { await model.signIn() } }
                } header: { Text("Connection & Upload Folders") }
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                Section("Encrypted Vault") {
                    SecureField("Passphrase or recovery key", text: $secret)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    Button("Unlock") { perform(.unlock) }
                    Button("Create New Vault") { confirmCreate = true }
                    Button("Recover with Recovery Key") { perform(.recover) }
                    Button("Change Passphrase") { perform(.changePassphrase) }
                    Button("Lock Vault") { perform(.lock) }
                    Button("Sync Now") {
                        Task { await model.sync(.synchronize); await loadFolders() }
                    }.disabled(!model.unlocked)
                    Button("Sign Out", role: .destructive) { perform(.logout) }
                }
                if let recovery = model.recovery {
                    Section("Save Your Recovery Key") {
                        Text(recovery).font(.system(.body, design: .monospaced))
                            .textSelection(.enabled).privacySensitive()
                        Text("Store this key somewhere safe before confirming. It is not written to settings or logs.")
                        Button("I Have Saved the Recovery Key") { perform(.confirmRecovery) }
                    }
                }
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
                                    }
                                }
                            }
                        }
                    }
                }
                Section {
                    Text("Sync is end-to-end encrypted. Local .txt files are not encrypted by the app. Sync runs while the app is open.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .disabled(model.busy)
            .navigationTitle("Encrypted Sync")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { secret = ""; dismiss() }
                }
            }
            .task { await load() }
            .onDisappear { secret = "" }
            .confirmationDialog("Create a new encrypted vault?", isPresented: $confirmCreate, titleVisibility: .visible) {
                Button("Create Vault") { perform(.create) }
            } message: {
                Text("Use a passphrase of at least 16 characters. Save the recovery key shown afterward.")
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

    private func perform(_ action: SyncAction) {
        let value = secret
        secret = ""
        Task { await model.sync(action, secret: value) }
    }
    private func load() async {
        do {
            let profile = try await model.call { try $0.syncProfile() }
            server = profile.server
            issuer = profile.issuer
            clientID = profile.clientId
            if !profile.scopes.isEmpty { scopes = profile.scopes }
            folders = Set(profile.folders)
            await loadFolders()
        } catch { model.errorMessage = error.localizedDescription }
    }
    private func loadFolders() async {
        do {
            remoteFolders = try await model.call { try $0.syncFolders() }.filter { $0.id != nil }
        } catch { model.errorMessage = error.localizedDescription }
    }
    private func configure() async {
        let profile = SyncProfile(server: server.trimmingCharacters(in: .whitespacesAndNewlines),
                                  issuer: issuer.trimmingCharacters(in: .whitespacesAndNewlines),
                                  clientId: clientID, scopes: scopes, folders: Array(folders))
        do {
            try await model.call { try $0.configureSync(profile: profile) }
            model.syncMessage = "Configuration saved. Sign in to continue."
        } catch { model.errorMessage = error.localizedDescription }
    }
}
