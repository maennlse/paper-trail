// SPDX-License-Identifier: GPL-3.0-or-later
import AuthenticationServices
import SwiftUI
import UIKit

/// A single worker serializes FFI calls and coordinates file-provider access.
/// Swift owns only platform services; Rust owns library/editor/sync state.
final class NativeHost: @unchecked Sendable {
    let client: PaperTrailClient
    private let worker = OperationQueue()
    private var scopedURLs: [URL] = []
    private var roots: [URL]
    private(set) var bookmarkWarning: String?
    private let bookmarkKey: String
    var lastSuccessfulSync: Date? {
        get { UserDefaults.standard.object(forKey: bookmarkKey + ".last-successful-sync") as? Date }
        set { UserDefaults.standard.set(newValue, forKey: bookmarkKey + ".last-successful-sync") }
    }

    init() throws {
        let manager = FileManager.default
        let documents: URL
        let support: URL
        if let profile = ProcessInfo.processInfo.environment["PAPERTRAIL_TEST_PROFILE"],
           let identifier = UUID(uuidString: profile) {
            let fixture = manager.urls(for: .cachesDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("UITests/\(identifier.uuidString)")
            documents = fixture.appendingPathComponent("Documents")
            support = fixture.appendingPathComponent("Support")
            bookmarkKey = "papertrail.test-bookmarks.\(identifier.uuidString)"
        } else {
            documents = manager.urls(for: .documentDirectory, in: .userDomainMask)[0]
            support = manager.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            bookmarkKey = "papertrail.folder-bookmarks"
        }
        roots = [documents.appendingPathComponent("Paper Trail"),
                 documents.appendingPathComponent("Paper Trail Synced")]
        for root in roots {
            try manager.createDirectory(at: root, withIntermediateDirectories: true)
        }
        worker.maxConcurrentOperationCount = 1
        worker.qualityOfService = .userInitiated
        var restored: [(String, URL)] = []
        var refreshed: [[String: Any]] = []
        for record in UserDefaults.standard.array(forKey: bookmarkKey) as? [[String: Any]] ?? [] {
            do {
                guard let data = record["data"] as? Data, let oldPath = record["path"] as? String else {
                    throw CocoaError(.coderReadCorrupt)
                }
                var stale = false
                let url = try URL(resolvingBookmarkData: data, options: [],
                                  relativeTo: nil, bookmarkDataIsStale: &stale)
                guard url.startAccessingSecurityScopedResource() else {
                    throw CocoaError(.fileReadNoPermission)
                }
                scopedURLs.append(url)
                roots.append(url)
                restored.append((oldPath, url))
                refreshed.append(["path": url.resolvingSymlinksInPath().path, "data": try url.bookmarkData(
                    options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil
                )])
            } catch {
                // Retain the bookmark so a temporarily unavailable provider can recover.
                refreshed.append(record)
                bookmarkWarning = "A saved Files folder is unavailable. Reconnect or disconnect it in Preferences."
            }
        }
        client = try PaperTrailClient(documents: documents.path, support: support.path)
        // Fixtures require both an isolated UUID profile and explicit opt-in.
        // Never seed, screenshot, or mutate the user's real library.
        if UUID(uuidString: ProcessInfo.processInfo.environment["PAPERTRAIL_TEST_PROFILE"] ?? "") != nil,
           ProcessInfo.processInfo.environment["PAPERTRAIL_TEST_FIXTURES"] == "1" {
            let marker = support.appendingPathComponent("fixtures-created")
            if !manager.fileExists(atPath: marker.path) {
                try client.addFolder(name: "Work Notes", color: "#779b82")
                try client.addFolder(name: "Personal", color: "#6f5bd6")
                let folder = URL(fileURLWithPath: try client.snapshot(folder: nil, query: "").defaultFolder)
                for (name, text) in [
                    ("weekend.md", "Weekend plans\nA long walk, a good book, and time to write."),
                    ("Reading List", "Reading list\nBooks and articles to come back to."),
                    ("ideas.txt", "Project ideas\nSmall improvements that make everyday tools feel better.")
                ] {
                    try text.write(to: folder.appendingPathComponent(name), atomically: true, encoding: .utf8)
                }
                try client.setPinned(path: folder.appendingPathComponent("ideas.txt").path, pinned: true)
                try Data([0, 255, 12]).write(to: folder.appendingPathComponent("sample.bin"))
                try Data().write(to: marker)
            }
        }
        for (old, url) in restored where old != url.resolvingSymlinksInPath().path {
            try client.relocateFolder(old: old, new: url.path)
        }
        UserDefaults.standard.set(refreshed, forKey: bookmarkKey)
    }

    deinit {
        for url in scopedURLs { url.stopAccessingSecurityScopedResource() }
    }

    func perform<T>(_ work: @escaping (PaperTrailClient) throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            worker.addOperation {
                // Collapse nested roots to avoid coordinating the same provider twice.
                let unique = Array(Set(self.roots))
                let roots = unique.filter { candidate in
                    FileManager.default.fileExists(atPath: candidate.path) && !unique.contains {
                        $0 != candidate && candidate.path.hasPrefix($0.path + "/")
                    }
                }
                let intents = roots.map { NSFileAccessIntent.writingIntent(with: $0, options: .forMerging) }
                let coordinator = NSFileCoordinator()
                // Callback uses a separate queue: never block the serial worker waiting
                // for a callback scheduled onto itself.
                let callbackQueue = OperationQueue()
                let done = DispatchSemaphore(value: 0)
                coordinator.coordinate(with: intents, queue: callbackQueue) { error in
                    defer { done.signal() }
                    if let error {
                        continuation.resume(throwing: error)
                    } else {
                        continuation.resume(with: Result { try work(self.client) })
                    }
                }
                done.wait()
            }
        }
    }

    func register(_ url: URL) async throws {
        guard url.startAccessingSecurityScopedResource() else {
            throw CocoaError(.fileReadNoPermission)
        }
        do {
            let data = try url.bookmarkData(options: .minimalBookmark,
                                            includingResourceValuesForKeys: nil, relativeTo: nil)
            try await perform { client in
                try client.registerFolder(path: url.path, makeDefault: true)
            }
            // All mutable host state is updated on its serial queue.
            await withCheckedContinuation { continuation in
                worker.addOperation {
                    self.scopedURLs.append(url)
                    self.roots.append(url)
                    var bookmarks = UserDefaults.standard.array(forKey: self.bookmarkKey) as? [[String: Any]] ?? []
                    let path = url.resolvingSymlinksInPath().path
                    bookmarks.removeAll { $0["path"] as? String == path }
                    bookmarks.append(["data": data, "path": path])
                    UserDefaults.standard.set(bookmarks, forKey: self.bookmarkKey)
                    continuation.resume()
                }
            }
        } catch {
            url.stopAccessingSecurityScopedResource()
            throw error
        }
    }

    func forget(_ path: String) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            worker.addOperation {
                do {
                    // This updates app-private metadata only, so it must also work
                    // when the provider cannot grant coordinated file access.
                    try self.client.forgetFolder(path: path)
                    let bookmarks = UserDefaults.standard.array(forKey: self.bookmarkKey) as? [[String: Any]] ?? []
                    UserDefaults.standard.set(bookmarks.filter { $0["path"] as? String != path }, forKey: self.bookmarkKey)
                    if self.scopedURLs.contains(where: { $0.resolvingSymlinksInPath().path == path }) {
                        self.roots.removeAll { $0.resolvingSymlinksInPath().path == path }
                    }
                    continuation.resume()
                } catch { continuation.resume(throwing: error) }
            }
        }
    }
}

@MainActor
final class LibraryModel: NSObject, ObservableObject, ASWebAuthenticationPresentationContextProviding {
    @Published var snapshot: LibrarySnapshot?
    @Published var selectedFolder: String?
    @Published var query = ""
    @Published var railOpen = false
    @Published var errorMessage: String?
    @Published var editorPath: String?
    @Published var editorText = ""
    @Published private(set) var editorTitle = ""
    @Published private(set) var editorSyntax: NoteSyntax?
    @Published private(set) var syntaxLanguages: [SyntaxLanguage] = []
    @Published private(set) var editorSave = EditorSaveState()
    @Published private(set) var closingEditor = false
    @Published private(set) var syncFeedback = SyncFeedback()
    @Published var busy = false
    @Published var syncing = false
    @Published var unlocked = false
    @Published var signedIn = false
    @Published var configured = false
    @Published var vaultExists: Bool?
    @Published var vaultPrompt: VaultPrompt?
    @Published var syncMessage = "Sync is optional. Local notes remain plaintext."
    @Published var recovery: String?
    @Published var storageWarning: String?
    private var host: NativeHost?
    private var starting = false
    private var saveTask: Task<Void, Never>?
    private var searchGeneration = 0
    private var authentication: ASWebAuthenticationSession?
    private var pendingSyncAction: SyncAction?
    private var automaticSyncBlocked = false
    private var operationCancelled = false
    private var stopGeneration = 0

    var colorScheme: ColorScheme? {
        switch snapshot?.appearance ?? "light" {
        case "dark": return .dark
        case "system": return nil
        default: return .light
        }
    }
    var folders: [FolderInfo] { snapshot?.folders ?? [] }
    var notes: [NoteInfo] { snapshot?.notes ?? [] }
    var title: String { folders.first { $0.path == selectedFolder }?.name ?? "All Notes" }
    var saveFailed: Bool { editorSave.phase == .failed }

    func call<T>(_ work: @escaping (PaperTrailClient) throws -> T) async throws -> T {
        guard let host else { throw CocoaError(.fileReadUnknown) }
        return try await host.perform(work)
    }

    func start() async {
        guard host == nil, !starting else { return }
        starting = true
        defer { starting = false }
        do {
            host = try await Task.detached { try NativeHost() }.value
            syntaxLanguages = try await call { $0.syntaxLanguages() }
            syncFeedback.lastSuccess = host?.lastSuccessfulSync
            storageWarning = host?.bookmarkWarning
            await refresh()
            // Restoring credentials must not prevent local-only use.
            updateSyncStatus()
            if configured && !automaticSyncBlocked {
                await sync(.restore)
            }
        } catch { errorMessage = error.localizedDescription }
    }

    func refresh() async {
        guard host != nil else { return }
        searchGeneration += 1
        let generation = searchGeneration
        let folder = selectedFolder
        let search = query
        let path = editorPath
        do {
            let (result, syntax) = try await call { client in
                let snapshot = try client.snapshot(folder: folder, query: search)
                return (snapshot, path.flatMap { try? client.noteSyntax(path: $0) })
            }
            if generation == searchGeneration {
                snapshot = result
                if editorPath == path, let syntax { editorSyntax = syntax }
            }
        } catch { errorMessage = error.localizedDescription }
    }

    func select(_ folder: String?) {
        selectedFolder = folder
        railOpen = false
        Task { await refresh() }
    }

    func open(_ path: String) async {
        do {
            let (text, syntax, title) = try await call { client in
                let text = try client.openNote(path: path)
                return (text, try client.noteSyntax(path: path), try client.editorTitle())
            }
            editorText = text
            editorTitle = title
            editorSyntax = syntax
            editorSave.reset()
            editorPath = path
        } catch { errorMessage = error.localizedDescription }
    }

    func compose() async {
        do {
            let folder = selectedFolder
            let path = try await call { try $0.createNote(folder: folder) }
            await open(path)
        } catch { errorMessage = error.localizedDescription }
    }

    func edited(_ text: String) {
        guard editorPath != nil, !closingEditor else { return }
        editorText = text
        editorSave.changed()
        let revision = editorSave.revision
        let path = editorPath
        saveTask?.cancel()
        saveTask = Task {
            // Queue edit immediately; only disk flushing is debounced.
            var saving: EditorSaveState.Token?
            do {
                guard !Task.isCancelled, self.editorSave.revision == revision,
                      self.editorPath == path else { return }
                let title = try await call { client in
                    try client.edit(text: text)
                    return try client.editorTitle()
                }
                if self.editorSave.revision == revision, self.editorPath == path {
                    self.editorTitle = title
                }
                try await Task.sleep(for: .milliseconds(350))
                guard !Task.isCancelled, self.editorSave.revision == revision,
                      self.editorPath == path else { return }
                let token = self.editorSave.begin()
                saving = token
                try await call { try $0.save() }
                self.editorSave.complete(token)
            } catch is CancellationError {
            } catch {
                if let token = saving {
                    self.editorSave.fail(token, message: error.localizedDescription)
                } else {
                    self.editorSave.failPendingEdit(revision, message: error.localizedDescription)
                }
            }
        }
    }

    func flush() async throws {
        saveTask?.cancel()
        let text = editorText
        let token = editorSave.begin()
        do {
            let title = try await call {
                try $0.edit(text: text)
                try $0.save()
                return try $0.editorTitle()
            }
            editorSave.complete(token)
            if token.revision == editorSave.revision { editorTitle = title }
        } catch {
            editorSave.fail(token, message: error.localizedDescription)
            throw error
        }
    }

    func closeEditor() async {
        guard !closingEditor else { return }
        closingEditor = true
        defer { closingEditor = false }
        do {
            try await flush()
            try await call { try $0.closeNote() }
            editorPath = nil
            await refresh()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func saveCopy() async {
        guard !closingEditor else { return }
        closingEditor = true
        defer { closingEditor = false }
        saveTask?.cancel()
        let token = editorSave.begin()
        do {
            let text = editorText
            let (path, syntax, title) = try await call { client in
                try client.edit(text: text)
                let path = try client.saveCopy()
                return (path, try client.noteSyntax(path: path), try client.editorTitle())
            }
            editorPath = path
            editorSyntax = syntax
            editorTitle = title
            editorSave.complete(token)
            await refresh()
        } catch {
            editorSave.fail(token, message: error.localizedDescription)
            errorMessage = error.localizedDescription
        }
    }

    func retrySave() async {
        do { try await flush() }
        catch { /* The persistent save banner retains the actionable error. */ }
    }

    func setEditorLanguage(_ language: String?) async -> Bool {
        guard let path = editorPath, !closingEditor else { return false }
        do {
            let syntax = try await call { client in
                try client.setNoteLanguage(path: path, language: language)
                return try client.noteSyntax(path: path)
            }
            if editorPath == path { editorSyntax = syntax }
            return true
        } catch { errorMessage = error.localizedDescription; return false }
    }

    func export(_ note: NoteInfo) async throws -> NoteExport {
        let bytes = try await call { try $0.exportNote(path: note.path) }
        return try NoteExport(filename: URL(fileURLWithPath: note.path).lastPathComponent,
                              data: Data(bytes))
    }

    func exportEditor() throws -> NoteExport {
        guard let path = editorPath else { throw CocoaError(.fileReadNoSuchFile) }
        // Export the current buffer, even after a failed save, without modifying
        // the original file, save status, or conflict baseline.
        return try NoteExport(filename: URL(fileURLWithPath: path).lastPathComponent,
                              data: Data(editorText.utf8))
    }

    func flushForBackground() {
        guard editorPath != nil else { return }
        let token = UIApplication.shared.beginBackgroundTask(withName: "Save note")
        Task {
            defer { UIApplication.shared.endBackgroundTask(token) }
            do { try await flush() }
            catch { errorMessage = error.localizedDescription }
        }
    }

    func pin(_ note: NoteInfo) async {
        do {
            let pinned = notes.first { $0.path == note.path }?.pinned ?? note.pinned
            try await call { try $0.setPinned(path: note.path, pinned: !pinned) }
            await refresh()
        } catch { errorMessage = error.localizedDescription }
    }

    func rename(_ note: NoteInfo, filename: String) async -> Bool {
        do {
            _ = try await call { try $0.renameNote(path: note.path, filename: filename) }
            await refresh()
            return true
        } catch { errorMessage = error.localizedDescription; return false }
    }

    func move(_ note: NoteInfo, to folder: String) async -> Bool {
        do {
            _ = try await call { try $0.moveNote(path: note.path, folder: folder) }
            await refresh()
            return true
        } catch { errorMessage = error.localizedDescription; return false }
    }

    func editFolder(_ folder: FolderInfo, name: String, color: String) async -> Bool {
        do {
            try await call { try $0.editFolder(path: folder.path, name: name, color: color) }
            await refresh()
            return true
        } catch { errorMessage = error.localizedDescription; return false }
    }

    func reorderFolder(_ source: String, relativeTo target: String, after: Bool) async {
        var paths = folders.map(\.path)
        guard source != target, paths.contains(source), paths.contains(target) else { return }
        paths.removeAll { $0 == source }
        guard let index = paths.firstIndex(of: target) else { return }
        paths.insert(source, at: index + (after ? 1 : 0))
        await persistFolderOrder(paths)
    }

    func moveFolder(_ path: String, by offset: Int) async {
        var paths = folders.map(\.path)
        guard let index = paths.firstIndex(of: path),
              paths.indices.contains(index + offset) else { return }
        paths.swapAt(index, index + offset)
        await persistFolderOrder(paths)
    }

    private func persistFolderOrder(_ paths: [String]) async {
        do {
            try await call { try $0.reorderFolders(paths: paths) }
            await refresh()
        } catch { errorMessage = error.localizedDescription }
    }

    func delete(_ note: NoteInfo) async {
        do {
            try await call { try $0.deleteNote(path: note.path) }
            await refresh()
        } catch { errorMessage = error.localizedDescription }
    }

    func addFolder(name: String, color: String) async {
        do {
            try await call { try $0.addFolder(name: name, color: color) }
            await refresh()
        } catch { errorMessage = error.localizedDescription }
    }

    func register(_ url: URL) async {
        do {
            try await host?.register(url)
            await refresh()
        } catch { errorMessage = error.localizedDescription }
    }

    func forget(_ path: String) async {
        do {
            try await host?.forget(path)
            if selectedFolder == path { selectedFolder = nil }
            storageWarning = nil
            await refresh()
        } catch { errorMessage = error.localizedDescription }
    }

    func preferences(scale: Double, appearance: String, editor: EditorPreferences) async -> Bool {
        do {
            try await call { try $0.preferences(fontScale: scale, appearance: appearance, editor: editor) }
            await refresh()
            return true
        } catch { errorMessage = error.localizedDescription; return false }
    }

    // These FFI status/cancellation methods only read atomics or signal a token;
    // unlike filesystem/network operations, they must bypass the serialized worker.
    func updateSyncStatus() {
        guard let status = host?.client.syncStatus() else { return }
        configured = status.configured
        signedIn = status.signedIn
        vaultExists = status.vaultExists
        unlocked = status.unlocked && pendingSyncAction == nil
    }

    func cancelOperation() {
        stopGeneration += 1
        operationCancelled = true
        automaticSyncBlocked = true
        authentication?.cancel()
        try? host?.client.cancelSync()
        syncMessage = "Cancelling… Waiting for the current request to finish."
    }

    private func finishOperation(autoSync: Bool = false) async {
        updateSyncStatus()
        busy = false
        if let pending = pendingSyncAction {
            pendingSyncAction = nil
            await sync(pending)
        } else if autoSync && !automaticSyncBlocked && !operationCancelled && unlocked {
            // Release the secret dialog before starting network work. A lock
            // requested in between must prevent this automatic follow-up.
            let generation = stopGeneration
            Task {
                guard !self.busy, !self.automaticSyncBlocked, self.pendingSyncAction == nil,
                      self.stopGeneration == generation else { return }
                await self.sync(.synchronize)
            }
        } else if operationCancelled {
            syncMessage = "Operation cancelled. Automatic sync paused; choose Sync Now to resume."
        }
    }

    @discardableResult
    func sync(_ action: SyncAction, secret: String = "", remember: Bool = false) async -> Bool {
        switch action {
        case .lock, .logout:
            if case .logout = action {
                syncFeedback = SyncFeedback()
                host?.lastSuccessfulSync = nil
            }
            stopGeneration += 1
            automaticSyncBlocked = true
            unlocked = false
            recovery = nil
            vaultPrompt = nil
            if busy {
                if case .logout? = pendingSyncAction {
                    // A queued sign-out must not be weakened to only locking.
                } else {
                    pendingSyncAction = action
                }
                cancelOperation()
                syncMessage = "Finishing the current operation, then locking/signing out…"
                return false
            }
        case .restore:
            guard !automaticSyncBlocked else { return false }
        default: break
        }
        guard !busy else { return false }
        switch action {
        case .unlock, .create, .recover, .changePassphrase, .synchronize:
            automaticSyncBlocked = false
        default: break
        }
        let wasUnlocked = unlocked
        busy = true
        if case .synchronize = action {
            syncing = true
            syncMessage = "Synchronizing…"
        }
        operationCancelled = false
        var succeeded = false
        var autoSync = false
        do {
            if syncing, editorPath != nil { try await flush() }
            guard pendingSyncAction == nil, !operationCancelled else {
                syncing = false
                await finishOperation()
                return false
            }
            let result = try await call { try $0.syncAction(action: action, secret: secret, remember: remember) }
            if pendingSyncAction == nil && !operationCancelled {
                syncMessage = [result.message, result.warning].compactMap { $0 }.joined(separator: "\n")
                if let value = result.recovery { recovery = value }
                if case .confirmRecovery = action { recovery = nil }
                autoSync = result.unlocked && !wasUnlocked && result.warning == nil
                succeeded = true
                if syncing {
                    syncFeedback.completed(at: Date(), succeeded: true,
                                           warning: result.warning, cancelled: false)
                    host?.lastSuccessfulSync = syncFeedback.lastSuccess
                }
            }
            if pendingSyncAction == nil && !operationCancelled { await refresh() }
        } catch {
            if pendingSyncAction == nil && !operationCancelled {
                if syncing {
                    syncMessage = "Sync did not finish."
                    syncFeedback.failure = error.localizedDescription
                } else {
                    errorMessage = error.localizedDescription
                }
            }
        }
        syncing = false
        await finishOperation(autoSync: autoSync)
        return succeeded
    }

    func prepareConfiguration(_ profile: SyncProfile) async -> SyncSetup? {
        guard !busy else { return nil }
        busy = true
        operationCancelled = false
        var setup: SyncSetup?
        do {
            let result = try await call { try $0.prepareSyncSetup(profile: profile) }
            if pendingSyncAction == nil && !operationCancelled {
                setup = result
            } else {
                try await call { try $0.discardSyncSetup(token: result.token) }
            }
        } catch {
            if pendingSyncAction == nil && !operationCancelled { errorMessage = error.localizedDescription }
        }
        await finishOperation()
        return setup
    }

    func confirmConfiguration(_ token: UInt64) async -> Bool {
        guard !busy else { return false }
        busy = true
        operationCancelled = false
        var succeeded = false
        do {
            try await call { try $0.confirmSyncSetup(token: token) }
            syncFeedback = SyncFeedback()
            host?.lastSuccessfulSync = nil
            syncMessage = "Configuration saved. Sign in/unlock if needed; selected folders will sync."
            succeeded = true
        } catch { errorMessage = error.localizedDescription }
        await finishOperation()
        return succeeded
    }

    func signIn() async {
        guard !busy else { return }
        let generation = stopGeneration
        busy = true
        operationCancelled = false
        do {
            let value = try await call { try $0.beginLogin() }
            guard pendingSyncAction == nil, !operationCancelled else {
                await finishOperation()
                return
            }
            guard let url = URL(string: value) else { throw CocoaError(.coderInvalidValue) }
            let session = ASWebAuthenticationSession(url: url, callbackURLScheme: "io.github.maennlse.paper-trail") { [weak self] callback, error in
                Task { @MainActor in
                    guard let self else { return }
                    var succeeded = false
                    if let callback, self.pendingSyncAction == nil, !self.operationCancelled {
                        do {
                            try await self.call { try $0.completeLogin(callback: callback.absoluteString) }
                            self.syncMessage = "Signed in. Unlock or create your encrypted vault."
                            succeeded = true
                        } catch {
                            if self.pendingSyncAction == nil && !self.operationCancelled {
                                self.errorMessage = error.localizedDescription
                            }
                        }
                    } else if let error,
                              self.pendingSyncAction == nil, !self.operationCancelled,
                              (error as? ASWebAuthenticationSessionError)?.code != .canceledLogin {
                        self.errorMessage = error.localizedDescription
                    }
                    self.authentication = nil
                    let shouldPrompt = succeeded && self.pendingSyncAction == nil && !self.operationCancelled
                    await self.finishOperation()
                    if shouldPrompt && !self.unlocked && self.signedIn && self.stopGeneration == generation {
                        self.vaultPrompt = self.vaultExists == false ? .create : .unlock
                    }
                }
            }
            session.presentationContextProvider = self
            session.prefersEphemeralWebBrowserSession = true
            authentication = session
            if !session.start() { throw CocoaError(.userCancelled) }
        } catch {
            if pendingSyncAction == nil && !operationCancelled { errorMessage = error.localizedDescription }
            authentication = nil
            await finishOperation()
        }
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first { $0.isKeyWindow } ?? ASPresentationAnchor()
    }
}
