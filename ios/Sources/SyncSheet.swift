// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct SyncSheet: View {
    @EnvironmentObject private var model: LibraryModel
    @Environment(\.dismiss) private var dismiss
    @State private var server = ""
    @State private var folders = Set<String>()
    @State private var remoteFolders: [SyncFolder] = []
    @State private var setup: SyncSetup?
    @State private var confirmTrust = false
    @State private var draftLoaded = false
    @State private var savedDraft: SyncProfile?
    @State private var folderDeletion: SyncFolderDeletion?
    @State private var folderMetadata: SyncFolder?

    private var profile: SyncProfile {
        SyncProfile(server: server, issuer: "", clientId: "",
                    scopes: "", folders: folders.sorted(), discoverAuth: true)
    }

    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
                Form {
                    statusSection {
                        if model.configured && !model.signedIn {
                            Task { await model.signIn() }
                        } else if model.signedIn && !model.unlocked {
                            model.vaultPrompt = .unlock
                        } else {
                            withAnimation { proxy.scrollTo("sync-connection", anchor: .top) }
                        }
                    }
                    connectionSection.id("sync-connection")
                    vaultSection
                    if let recovery = model.recovery { recoverySection(recovery) }
                    remoteFolderSection
                    Section {
                        Text("Sync is end-to-end encrypted. Local note files are not encrypted by the app. Sync runs while the app is open.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
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
            .onChange(of: model.busy) { _, busy in
                if !busy { Task { await loadFolders() } }
            }
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
            .confirmationDialog("Delete Folder from Server?", isPresented: Binding(
                get: { folderDeletion != nil },
                set: { if !$0 { folderDeletion = nil } }
            ), titleVisibility: .visible, presenting: folderDeletion) { target in
                Button("Delete from Server", role: .destructive) {
                    folderDeletion = nil
                    Task {
                        await model.sync(.deleteFolder(id: target.id))
                        await loadFolders()
                    }
                }
                .disabled(!canDeleteFolder)
                Button("Cancel", role: .cancel) { folderDeletion = nil }
            } message: { target in
                Text(target.message)
            }
            .confirmationDialog("Accept Remote Folder Name and Color?", isPresented: Binding(
                get: { folderMetadata != nil },
                set: { if !$0 { folderMetadata = nil } }
            ), titleVisibility: .visible, presenting: folderMetadata) { folder in
                Button("Use Remote Name and Color") {
                    folderMetadata = nil
                    if let id = folder.id {
                        Task {
                            await model.acceptRemoteFolder(id)
                            await loadFolders()
                        }
                    }
                }.disabled(model.busy || model.recovery != nil)
                Button("Cancel", role: .cancel) { folderMetadata = nil }
            } message: { folder in
                Text("This folder changed on both devices. Use the remote name “\(folder.name)” and color on the next sync? Local directories and notes are kept. You can rename it again afterward.\n\nFolder ID: \(folder.id ?? "")")
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

    private func statusSection(settings: @escaping () -> Void) -> some View {
        Section("Status") {
            Label(model.syncMessage, systemImage: model.unlocked ? "lock.open" : "lock.fill")
            SyncFeedbackView(settings: settings)
            if model.busy {
                if !model.syncing { ProgressView() }
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
            Text("Enter your HTTPS sync server URL. Authentication settings are discovered automatically; you will be asked before trusting a new identity provider.")
                .font(.caption).foregroundStyle(.secondary)
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
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(folder.name)
                            Spacer()
                            if folder.deleted {
                                Text("Local only").foregroundStyle(.secondary)
                            } else if folder.conflict {
                                Button("Review Name and Color") { folderMetadata = folder }
                                    .buttonStyle(.borderless)
                                    .disabled(model.busy || model.recovery != nil)
                            } else if folder.selected {
                                Image(systemName: "checkmark")
                            } else if let id = folder.id {
                                Button("Download") {
                                    Task {
                                        await model.downloadRemoteFolder(id)
                                        await loadFolders()
                                    }
                                }
                                .buttonStyle(.borderless)
                                .disabled(model.busy || model.recovery != nil || !model.signedIn || !model.unlocked)
                            }
                            if let target = SyncFolderDeletion(
                                id: folder.id, name: folder.name,
                                published: folder.published, deleted: folder.deleted
                            ) {
                                Button(role: .destructive) { folderDeletion = target } label: {
                                    Image(systemName: "trash")
                                }
                                .buttonStyle(.borderless)
                                .accessibilityLabel("Delete \(folder.name) from Server")
                                .accessibilityIdentifier("sync-delete-\(target.id)")
                                .disabled(!canDeleteFolder)
                            }
                        }
                        Text(folder.path ?? "Automatic download directory")
                            .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                        if let storedPath = folder.storedPath {
                            Text("Current directory: \(storedPath) (mapping change pending)")
                                .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                        }
                        if let id = folder.id {
                            Text("ID: \(id)")
                                .font(.caption2).foregroundStyle(.secondary).textSelection(.enabled)
                        }
                        if folder.deleted {
                            Text("Removed from server. Local files are kept and will not be uploaded again automatically.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }

    private var canDeleteFolder: Bool {
        SyncFolderDeletion.isAvailable(signedIn: model.signedIn, unlocked: model.unlocked,
                                       busy: model.busy, hasRecovery: model.recovery != nil)
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
            folders = Set(saved.folders)
            model.updateSyncStatus()
            savedDraft = profile
            draftLoaded = true
            await loadFolders()
        } catch { model.errorMessage = error.localizedDescription }
    }

    private func loadFolders() async {
        do {
            remoteFolders = try await model.call(coordinatingFiles: false) {
                try $0.syncFolders()
            }.filter { $0.id != nil }
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
