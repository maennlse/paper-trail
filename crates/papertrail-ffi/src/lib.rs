// SPDX-License-Identifier: GPL-3.0-or-later
//! Native client boundary. All paths are scoped to explicitly registered folders.
use papertrail_core::{colors, note::sort_notes, Session, Settings, Store};
use papertrail_storage::{Repository, SettingsFile};
use papertrail_sync::{
    auth::LoginFlow,
    control::{Cancellation, Command, PassphraseAction, Service},
    Config,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Instant, UNIX_EPOCH},
};
use zeroize::Zeroizing;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ClientError {
    #[error("{message}")]
    Operation { message: String },
}
impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        Self::Operation {
            message: error.to_string(),
        }
    }
}
type Result<T> = std::result::Result<T, ClientError>;
fn error(message: &str) -> ClientError {
    io::Error::other(message).into()
}

#[derive(Clone, uniffi::Record)]
pub struct FolderInfo {
    pub path: String,
    pub name: String,
    pub color: String,
    pub badge: String,
}
#[derive(Clone, uniffi::Record)]
pub struct NoteInfo {
    pub path: String,
    pub title: String,
    pub modified: u64,
    pub folder: String,
    pub folder_name: String,
    pub color: String,
    pub pinned: bool,
}
#[derive(uniffi::Record)]
pub struct LibrarySnapshot {
    pub folders: Vec<FolderInfo>,
    pub notes: Vec<NoteInfo>,
    pub default_folder: String,
    pub font_scale: f64,
    pub appearance: String,
    pub unavailable_folders: Vec<String>,
}
#[derive(uniffi::Record)]
pub struct SyncProfile {
    pub server: String,
    pub issuer: String,
    pub client_id: String,
    pub scopes: String,
    pub folders: Vec<String>,
    pub discover_auth: bool,
}
#[derive(uniffi::Record)]
pub struct SyncSetup {
    pub token: u64,
    pub needs_confirmation: bool,
    pub trust_description: String,
}
#[derive(uniffi::Record)]
pub struct SyncStatus {
    pub configured: bool,
    pub signed_in: bool,
    pub unlocked: bool,
    pub vault_exists: Option<bool>,
}
#[derive(uniffi::Record)]
pub struct SyncResult {
    pub message: String,
    pub warning: Option<String>,
    pub recovery: Option<String>,
    pub unlocked: bool,
}
#[derive(uniffi::Record)]
pub struct SyncFolder {
    pub id: Option<String>,
    pub path: Option<String>,
    pub name: String,
    pub selected: bool,
}
#[derive(uniffi::Enum)]
pub enum SyncAction {
    Restore,
    Status,
    Unlock,
    Create,
    Recover,
    ConfirmRecovery,
    ChangePassphrase,
    Lock,
    Logout,
    Synchronize,
}

struct State {
    settings: Settings,
    session: Session,
}
struct PendingSetup {
    token: u64,
    baseline: Option<Config>,
    candidate: Config,
}
#[derive(uniffi::Object)]
pub struct PaperTrailClient {
    state: Mutex<State>,
    settings_file: SettingsFile,
    home: PathBuf,
    service: Service,
    login: Mutex<Option<LoginFlow>>,
    setup_generation: AtomicU64,
    pending_setup: Mutex<Option<PendingSetup>>,
    operation: Mutex<Option<Cancellation>>,
}

impl PaperTrailClient {
    fn state(&self) -> Result<MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| error("Client unavailable"))
    }
    fn persist(&self, state: &mut State, updated: Settings) -> Result<()> {
        state.settings = self
            .settings_file
            .save_merged(&updated, &state.settings, &self.home)?;
        Ok(())
    }
    fn folder(&self, settings: &Settings, path: &str) -> Result<PathBuf> {
        let path = fs::canonicalize(path)?;
        if settings
            .note_folders
            .iter()
            .any(|p| fs::canonicalize(p).is_ok_and(|registered| registered == path))
        {
            Ok(path)
        } else {
            Err(error("Folder is not registered"))
        }
    }
    fn note(&self, settings: &Settings, path: &str) -> Result<PathBuf> {
        let path = PathBuf::from(path);
        let parent = path.parent().ok_or_else(|| error("Missing folder"))?;
        let folder = self.folder(settings, &parent.to_string_lossy())?;
        let target = fs::canonicalize(&path)?;
        if target.parent() != Some(folder.as_path())
            || !target.is_file()
            || fs::symlink_metadata(&path)?.file_type().is_symlink()
        {
            return Err(error(
                "Only regular files inside registered folders are allowed",
            ));
        }
        Ok(target)
    }

    fn require_text(path: &Path) -> Result<()> {
        let bytes = fs::read(path)?;
        if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            return Err(error("This file is not UTF-8 text and cannot be edited. Its contents have not been changed."));
        }
        Ok(())
    }

    fn configuration(&self, profile: SyncProfile, previous: Option<Config>) -> Result<Config> {
        let state = self.state()?;
        let folders = profile
            .folders
            .iter()
            .map(|p| self.folder(&state.settings, p))
            .collect::<Result<Vec<_>>>()?;
        let mut config = previous.unwrap_or_default();
        for choice in self.service.folder_choices(&state.settings)? {
            if let (Some(id), Some(path)) = (choice.id, choice.path) {
                if folders
                    .iter()
                    .any(|selected| papertrail_sync::same_directory(selected, &path))
                {
                    config.excluded_folders.remove(&id);
                    config.selected_folders.insert(id);
                } else {
                    config.selected_folders.remove(&id);
                    config.excluded_folders.insert(id);
                }
            }
        }
        config.server = profile.server.trim().trim_end_matches('/').into();
        config.issuer = profile.issuer.trim().into();
        config.client_id = profile.client_id;
        config.scopes = profile.scopes;
        config.discover_auth = profile.discover_auth;
        config.folders = folders;
        config
            .download_dir
            .get_or_insert_with(|| self.home.join("Paper Trail Synced"));
        Ok(config)
    }

    fn prepare_setup(
        &self,
        profile: SyncProfile,
        discover: impl FnOnce(Config) -> io::Result<Config>,
    ) -> Result<SyncSetup> {
        let generation = self.setup_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self
            .pending_setup
            .lock()
            .map_err(|_| error("Setup unavailable"))? = None;
        let baseline = self.service.saved_config()?;
        let candidate = self.configuration(profile, baseline.clone())?;
        let candidate = if candidate.discover_auth {
            discover(candidate)?
        } else {
            candidate
        };
        candidate.validate()?;
        if self.service.saved_config()? != baseline {
            return Err(error("Sync configuration changed; review setup again"));
        }
        let preview = SyncSetup {
            token: generation,
            needs_confirmation: candidate.discover_auth
                && papertrail_sync::auth::config::needs_confirmation(baseline.as_ref(), &candidate),
            trust_description: papertrail_sync::auth::config::trust_description(
                baseline.as_ref(),
                &candidate,
            ),
        };
        let mut pending = self
            .pending_setup
            .lock()
            .map_err(|_| error("Setup unavailable"))?;
        if self.setup_generation.load(Ordering::Acquire) != generation {
            return Err(error("Setup was superseded; review the current server"));
        }
        *pending = Some(PendingSetup {
            token: generation,
            baseline,
            candidate,
        });
        Ok(preview)
    }
}

fn passphrase_command(
    action: PassphraseAction,
    secret: Zeroizing<String>,
    remember: bool,
) -> Command {
    Command::Passphrase {
        action,
        passphrase: secret,
        remember,
    }
}

#[uniffi::export]
impl PaperTrailClient {
    #[uniffi::constructor]
    pub fn new(documents: String, support: String) -> Result<Arc<Self>> {
        let documents = PathBuf::from(documents);
        let support = PathBuf::from(support);
        if !documents.is_absolute() || !support.is_absolute() {
            return Err(error("Absolute app directories required"));
        }
        fs::create_dir_all(&documents)?;
        let documents = fs::canonicalize(documents)?;
        let default = documents.join("Paper Trail");
        fs::create_dir_all(&default)?;
        fs::create_dir_all(&support)?;
        let settings_file = SettingsFile {
            path: support.join("settings.json"),
        };
        let first_run = !settings_file.path.exists();
        let mut settings = settings_file.load(default, &documents)?;
        if first_run {
            settings.theme_mode = "light".into();
            settings
                .folder_colors
                .insert(settings.notes_dir.clone(), "preset-5".into());
        }
        // Materialize defaults before sync so metadata updates are persisted.
        let mut settings = settings_file.save_merged(&settings, &settings, &documents)?;
        let service = Service::new(&settings_file.path);
        // Container paths can change after reinstall/restore or a sideload update.
        let location_file = support.join("documents-path");
        if let Ok(previous) = fs::read_to_string(&location_file) {
            let previous = PathBuf::from(previous);
            if previous != documents {
                let mut updated = settings.clone();
                updated.rename_folder(&previous, &documents);
                service.relocate(&previous, &documents)?;
                settings = settings_file.save_merged(&updated, &settings, &documents)?;
            }
        }
        papertrail_storage::atomic_write(&location_file, documents.to_string_lossy().as_bytes())?;
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                settings,
                session: Session::default(),
            }),
            settings_file,
            home: documents,
            service,
            login: Mutex::new(None),
            setup_generation: AtomicU64::new(0),
            pending_setup: Mutex::new(None),
            operation: Mutex::new(None),
        }))
    }

    /// A query always searches full contents across all registered folders.
    pub fn snapshot(&self, folder: Option<String>, query: String) -> Result<LibrarySnapshot> {
        let state = self.state()?;
        let settings = &state.settings;
        let selected = folder
            .as_deref()
            .map(|p| {
                if settings
                    .note_folders
                    .iter()
                    .any(|registered| papertrail_sync::same_directory(registered, Path::new(p)))
                {
                    Ok(PathBuf::from(p))
                } else {
                    Err(error("Folder is not registered"))
                }
            })
            .transpose()?;
        let mut folders = Vec::new();
        let mut notes = Vec::new();
        let mut unavailable_folders = Vec::new();
        for path in &settings.note_folders {
            folders.push(FolderInfo {
                badge: colors::badge(&settings.folder_name(path)),
                path: path.to_string_lossy().into(),
                name: settings.folder_name(path),
                color: colors::hex(
                    settings
                        .folder_colors
                        .get(path)
                        .map(String::as_str)
                        .unwrap_or("preset-5"),
                ),
            });
            if query.trim().is_empty()
                && selected
                    .as_ref()
                    .is_some_and(|p| !papertrail_sync::same_directory(p, path))
            {
                continue;
            }
            // Keep available local notes usable, but make provider failures explicit.
            let entries = match fs::read_dir(path) {
                Ok(entries) => entries,
                Err(e) => {
                    unavailable_folders.push(format!("{}: {e}", settings.folder_name(path)));
                    continue;
                }
            };
            for entry in entries {
                let path = entry?.path();
                if !path.is_file() || fs::symlink_metadata(&path)?.file_type().is_symlink() {
                    continue;
                }
                let bytes = match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        unavailable_folders.push(format!("{}: {e}", path.display()));
                        continue;
                    }
                };
                // Keep binary files visible, but never use lossy decoding in the editor.
                let text = std::str::from_utf8(&bytes)
                    .ok()
                    .filter(|_| !bytes.contains(&0))
                    .unwrap_or("");
                let note = papertrail_core::Note::from_text(
                    path.clone(),
                    text.replace("\r\n", "\n").replace('\r', "\n"),
                    fs::metadata(&path)?.modified()?,
                );
                if note.matches(&query) {
                    notes.push(note);
                }
            }
        }
        sort_notes(&mut notes, &settings.pinned_notes);
        Ok(LibrarySnapshot {
            folders,
            notes: notes
                .into_iter()
                .map(|note| {
                    let parent = note.path.parent().unwrap_or(Path::new(""));
                    NoteInfo {
                        path: note.path.to_string_lossy().into(),
                        title: note.title,
                        modified: note
                            .modified
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        folder: parent.to_string_lossy().into(),
                        folder_name: settings.folder_name(parent),
                        color: colors::hex(
                            settings
                                .folder_colors
                                .get(parent)
                                .map(String::as_str)
                                .unwrap_or("preset-5"),
                        ),
                        pinned: settings.pinned_notes.contains(&note.path),
                    }
                })
                .collect(),
            default_folder: settings.notes_dir.to_string_lossy().into(),
            font_scale: settings.font_scale,
            appearance: settings.theme_mode.clone(),
            unavailable_folders,
        })
    }

    pub fn create_note(&self, folder: Option<String>) -> Result<String> {
        let state = self.state()?;
        let folder = self.folder(
            &state.settings,
            folder
                .as_deref()
                .unwrap_or(&state.settings.notes_dir.to_string_lossy()),
        )?;
        Ok(Repository.create(&folder)?.path.to_string_lossy().into())
    }
    pub fn open_note(&self, path: String) -> Result<String> {
        let mut state = self.state()?;
        let path = self.note(&state.settings, &path)?;
        let _activity = self.service.local()?;
        Self::require_text(&path)?;
        state.session.open(&Repository, &path)?;
        Ok(state.session.text.clone())
    }
    pub fn edit(&self, text: String) -> Result<()> {
        self.state()?.session.edit(text, Instant::now());
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        let mut state = self.state()?;
        let _activity = self.service.local()?;
        state.session.flush(&Repository)?;
        Ok(())
    }
    pub fn close_note(&self) -> Result<()> {
        let mut state = self.state()?;
        let _activity = self.service.local()?;
        state.session.close(&Repository)?;
        Ok(())
    }
    /// Explicit recovery preserves unsaved edits and leaves the external version intact.
    pub fn save_copy(&self) -> Result<String> {
        let mut state = self.state()?;
        let current = state
            .session
            .current
            .as_ref()
            .ok_or_else(|| error("No open note"))?;
        let folder = current
            .path
            .parent()
            .ok_or_else(|| error("Missing folder"))?;
        let _activity = self.service.local()?;
        let note = Repository.create(folder)?;
        Repository.save(&note.path, &state.session.text)?;
        state.session = Session::default();
        state.session.open(&Repository, &note.path)?;
        Ok(note.path.to_string_lossy().into())
    }
    pub fn set_pinned(&self, path: String, pinned: bool) -> Result<()> {
        let mut state = self.state()?;
        let path = self.note(&state.settings, &path)?;
        let mut updated = state.settings.clone();
        updated.set_pinned(&path, pinned);
        self.persist(&mut state, updated)
    }
    pub fn rename_note(&self, path: String, filename: String) -> Result<String> {
        let mut state = self.state()?;
        let path = self.note(&state.settings, &path)?;
        if state
            .session
            .current
            .as_ref()
            .is_some_and(|n| n.path == path)
        {
            return Err(error("Close the editor before renaming this note"));
        }
        let _activity = self.service.local()?;
        let mut filename = papertrail_core::note::normalize_filename(&filename, "untitled-note");
        if Path::new(&filename).extension().is_none() {
            if let Some(extension) = path.extension() {
                filename.push('.');
                filename.push_str(&extension.to_string_lossy());
            }
        }
        let renamed = Repository.rename(&path, &filename)?;
        let mut updated = state.settings.clone();
        updated.rename_note(&path, &renamed.path);
        self.persist(&mut state, updated)?;
        Ok(renamed.path.to_string_lossy().into())
    }

    pub fn reorder_folders(&self, paths: Vec<String>) -> Result<()> {
        let mut state = self.state()?;
        let mut updated = state.settings.clone();
        if !updated.reorder(paths.into_iter().map(PathBuf::from).collect()) {
            return Err(error(
                "Folder order must contain each registered folder exactly once",
            ));
        }
        self.persist(&mut state, updated)
    }
    pub fn move_note(&self, path: String, folder: String) -> Result<String> {
        let mut state = self.state()?;
        let path = self.note(&state.settings, &path)?;
        let folder = self.folder(&state.settings, &folder)?;
        if state
            .session
            .current
            .as_ref()
            .is_some_and(|n| n.path == path)
        {
            return Err(error("Close the editor before moving this note"));
        }
        let _activity = self.service.local()?;
        let moved = Repository.move_note(&path, &folder)?;
        let mut updated = state.settings.clone();
        updated.rename_note(&path, &moved.path);
        self.persist(&mut state, updated)?;
        Ok(moved.path.to_string_lossy().into())
    }
    pub fn set_folder_color(&self, path: String, color: String) -> Result<()> {
        let mut state = self.state()?;
        let folder = self.folder(&state.settings, &path)?;
        let mut updated = state.settings.clone();
        updated
            .folder_colors
            .insert(folder, colors::normalize(&color.into()));
        self.persist(&mut state, updated)
    }
    /// Update presentation metadata, not the provider directory or its bookmark.
    pub fn edit_folder(&self, path: String, name: String, color: String) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(error(
                "Enter a nonempty folder display name without control characters",
            ));
        }
        let mut state = self.state()?;
        let path = PathBuf::from(path);
        if !state.settings.note_folders.contains(&path) {
            return Err(error("Folder is not registered"));
        }
        let mut updated = state.settings.clone();
        updated.folder_labels.insert(path.clone(), name.into());
        updated
            .folder_colors
            .insert(path, colors::normalize(&color.into()));
        self.persist(&mut state, updated)
    }

    /// Export bytes unchanged, including files not supported by the text editor.
    pub fn export_note(&self, path: String) -> Result<Vec<u8>> {
        let state = self.state()?;
        let path = self.note(&state.settings, &path)?;
        Ok(fs::read(path)?)
    }
    pub fn delete_note(&self, path: String) -> Result<()> {
        let mut state = self.state()?;
        let path = self.note(&state.settings, &path)?;
        if state
            .session
            .current
            .as_ref()
            .is_some_and(|n| n.path == path)
        {
            return Err(error("Close the editor before deleting this note"));
        }
        let _activity = self.service.local()?;
        Repository.delete(&path)?;
        let mut updated = state.settings.clone();
        updated.delete_note(&path);
        self.persist(&mut state, updated)
    }
    pub fn add_folder(&self, name: String, color: String) -> Result<()> {
        let mut state = self.state()?;
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
            return Err(error("Enter a folder name without path separators"));
        }
        let folder = self.home.join("Paper Trail").join(name);
        fs::create_dir(&folder)?;
        let mut updated = state.settings.clone();
        updated.note_folders.push(folder.clone());
        updated
            .folder_colors
            .insert(folder, colors::normalize(&color.into()));
        self.persist(&mut state, updated)
    }
    /// Rebind a refreshed security-scoped bookmark without losing path-keyed metadata.
    pub fn relocate_folder(&self, old: String, new: String) -> Result<()> {
        let new = fs::canonicalize(new)?;
        let mut state = self.state()?;
        let old = PathBuf::from(old);
        if old == new {
            return Ok(());
        }
        if !state.settings.note_folders.contains(&old) {
            return Err(error("Old folder is not registered"));
        }
        self.service.relocate(&old, &new)?;
        let mut updated = state.settings.clone();
        updated.rename_folder(&old, &new);
        self.persist(&mut state, updated)
    }
    /// The native host must retain security-scoped access for this path.
    pub fn register_folder(&self, path: String, make_default: bool) -> Result<()> {
        let folder = fs::canonicalize(path)?;
        if !folder.is_dir() {
            return Err(error("Choose a directory"));
        }
        let mut state = self.state()?;
        let mut updated = state.settings.clone();
        if !updated.note_folders.contains(&folder) {
            updated.note_folders.push(folder.clone());
        }
        if make_default {
            updated.notes_dir = folder;
        }
        self.persist(&mut state, updated)
    }
    /// Disconnect a provider without deleting any files or shared note metadata.
    pub fn forget_folder(&self, path: String) -> Result<()> {
        let mut state = self.state()?;
        if state.settings.note_folders.len() <= 1 {
            return Err(error("Keep at least one notes folder"));
        }
        let path = PathBuf::from(path);
        if state
            .session
            .current
            .as_ref()
            .is_some_and(|n| n.path.starts_with(&path))
        {
            return Err(error("Close the editor before disconnecting this folder"));
        }
        self.service.pause_folder(&path)?;
        let mut updated = state.settings.clone();
        updated.remove_folder(&path);
        self.persist(&mut state, updated)
    }
    pub fn preferences(&self, font_scale: f64, appearance: String) -> Result<()> {
        if !font_scale.is_finite() || !["light", "dark", "system"].contains(&appearance.as_str()) {
            return Err(error("Invalid appearance settings"));
        }
        let mut state = self.state()?;
        let mut updated = state.settings.clone();
        updated.font_scale = font_scale.clamp(0.7, 2.5);
        updated.theme_mode = appearance;
        self.persist(&mut state, updated)
    }
    pub fn sync_profile(&self) -> Result<SyncProfile> {
        let config = self.service.saved_config()?.unwrap_or_default();
        let folders = self
            .service
            .folder_choices(&self.state()?.settings)?
            .into_iter()
            .filter(|choice| choice.selected)
            .filter_map(|choice| choice.path)
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(SyncProfile {
            server: config.server,
            issuer: config.issuer,
            client_id: config.client_id,
            scopes: config.scopes,
            discover_auth: config.discover_auth,
            folders,
        })
    }
    pub fn configure_sync(&self, profile: SyncProfile) -> Result<()> {
        if profile.discover_auth {
            return Err(error(
                "Discovered authentication requires setup review and confirmation",
            ));
        }
        self.service
            .configure(self.configuration(profile, self.service.saved_config()?)?)?;
        Ok(())
    }
    /// Discovery never persists settings or starts authentication.
    pub fn prepare_sync_setup(&self, profile: SyncProfile) -> Result<SyncSetup> {
        self.prepare_setup(profile, papertrail_sync::auth::config::prepare)
    }
    pub fn confirm_sync_setup(&self, token: u64) -> Result<()> {
        let mut pending = self
            .pending_setup
            .lock()
            .map_err(|_| error("Setup unavailable"))?;
        let setup = pending
            .as_ref()
            .filter(|setup| setup.token == token)
            .ok_or_else(|| error("Setup expired; review the current server again"))?;
        if self.setup_generation.load(Ordering::Acquire) != token {
            return Err(error("Setup was superseded; review the current server"));
        }
        if self.service.saved_config()? != setup.baseline {
            return Err(error("Sync configuration changed; review setup again"));
        }
        // Revalidate folder access after a confirmation dialog/provider change.
        for folder in &setup.candidate.folders {
            self.folder(&self.state()?.settings, &folder.to_string_lossy())?;
        }
        self.service.configure(setup.candidate.clone())?;
        pending.take();
        Ok(())
    }
    pub fn discard_sync_setup(&self, token: u64) -> Result<()> {
        let mut pending = self
            .pending_setup
            .lock()
            .map_err(|_| error("Setup unavailable"))?;
        if pending.as_ref().is_some_and(|setup| setup.token == token) {
            pending.take();
        }
        Ok(())
    }
    pub fn sync_status(&self) -> SyncStatus {
        SyncStatus {
            configured: self.service.configured(),
            signed_in: self.service.signed_in(),
            unlocked: self.service.unlocked(),
            vault_exists: self.service.vault_exists(),
        }
    }
    /// A control-plane signal: safe to call while the serialized worker is busy.
    pub fn cancel_sync(&self) -> Result<()> {
        if let Some(cancel) = self
            .operation
            .lock()
            .map_err(|_| error("Sync unavailable"))?
            .as_ref()
        {
            cancel.cancel();
        }
        Ok(())
    }
    pub fn begin_login(&self) -> Result<String> {
        let config = self
            .service
            .saved_config()?
            .ok_or_else(|| error("Configure sync first"))?;
        papertrail_sync::auth::config::check(&config)?;
        let (flow, url) =
            LoginFlow::begin(&config, "io.github.maennlse.paper-trail://oauth/callback")?;
        *self.login.lock().map_err(|_| error("Login unavailable"))? = Some(flow);
        Ok(url)
    }
    pub fn complete_login(&self, callback: String) -> Result<()> {
        let flow = self
            .login
            .lock()
            .map_err(|_| error("Login unavailable"))?
            .take()
            .ok_or_else(|| error("Start sign-in again"))?;
        self.service.install_tokens(flow.complete(&callback)?)?;
        Ok(())
    }
    pub fn sync_folders(&self) -> Result<Vec<SyncFolder>> {
        Ok(self
            .service
            .folder_choices(&self.state()?.settings)?
            .into_iter()
            .map(|f| SyncFolder {
                id: f.id,
                path: f.path.map(|p| p.to_string_lossy().into()),
                name: f.name,
                selected: f.selected,
            })
            .collect())
    }
    pub fn accept_sync_folder(&self, id: String) -> Result<()> {
        self.service
            .accept_remote_folder(&id, &self.state()?.settings)?;
        Ok(())
    }
    pub fn sync_action(
        &self,
        action: SyncAction,
        secret: String,
        remember: bool,
    ) -> Result<SyncResult> {
        let secret = Zeroizing::new(secret);
        let mut state = self.state()?;
        let command = match action {
            SyncAction::Restore => Command::Restore,
            SyncAction::Status => Command::Status,
            SyncAction::Unlock => passphrase_command(PassphraseAction::Unlock, secret, remember),
            SyncAction::Create => passphrase_command(PassphraseAction::Create, secret, remember),
            SyncAction::Recover => Command::Recover(secret),
            SyncAction::ConfirmRecovery => Command::ConfirmRecovery,
            SyncAction::ChangePassphrase => {
                passphrase_command(PassphraseAction::Change, secret, remember)
            }
            SyncAction::Lock => Command::Lock,
            SyncAction::Logout => Command::Logout,
            SyncAction::Synchronize => {
                state.session.flush(&Repository)?;
                Command::Sync {
                    settings: Box::new(state.settings.clone()),
                    protected: state
                        .session
                        .current
                        .iter()
                        .map(|n| n.path.clone())
                        .collect(),
                }
            }
        };
        let cancel = Cancellation::default();
        *self
            .operation
            .lock()
            .map_err(|_| error("Sync unavailable"))? = Some(cancel.clone());
        let result = self
            .service
            .submit_cancellable(command, cancel)
            .recv()
            .map_err(|_| error("Sync worker stopped"));
        *self
            .operation
            .lock()
            .map_err(|_| error("Sync unavailable"))? = None;
        let outcome = result??;
        state.settings = self
            .settings_file
            .load(state.settings.notes_dir.clone(), &self.home)?;
        Ok(SyncResult {
            message: outcome.message,
            warning: outcome.warning,
            recovery: outcome.recovery.map(|s| s.to_string()),
            unlocked: outcome.unlocked,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client(root: &Path) -> Arc<PaperTrailClient> {
        PaperTrailClient::new(
            root.join("Documents").to_string_lossy().into(),
            root.join("Support").to_string_lossy().into(),
        )
        .unwrap()
    }
    fn setup_profile(app: &PaperTrailClient, discover_auth: bool) -> SyncProfile {
        SyncProfile {
            server: "https://sync.example".into(),
            issuer: "https://manual.example".into(),
            client_id: "manual-native".into(),
            scopes: "openid".into(),
            folders: vec![app.snapshot(None, "".into()).unwrap().default_folder],
            discover_auth,
        }
    }
    fn discovery(mut config: Config) -> io::Result<Config> {
        config.issuer = "https://identity.example".into();
        config.client_id = "discovered-native".into();
        config.scopes = "openid offline_access api:notes".into();
        config.discover_auth = true;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn discovery_requires_explicit_trust_and_persists_the_discovery_mode() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let preview = app
            .prepare_setup(setup_profile(&app, true), discovery)
            .unwrap();
        assert!(preview.needs_confirmation);
        assert!(preview
            .trust_description
            .contains("https://identity.example"));
        assert!(!app.sync_status().configured);
        assert!(!app.sync_status().signed_in);
        app.confirm_sync_setup(preview.token).unwrap();
        let saved = app.sync_profile().unwrap();
        assert!(saved.discover_auth);
        assert_eq!(saved.client_id, "discovered-native");
        let preview = app
            .prepare_setup(setup_profile(&app, true), discovery)
            .unwrap();
        assert!(
            !preview.needs_confirmation,
            "unchanged issuer is already trusted"
        );
    }

    #[test]
    fn cancelled_superseded_and_stale_discovery_cannot_reconfigure_sync() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let cancelled = app
            .prepare_setup(setup_profile(&app, true), discovery)
            .unwrap();
        app.discard_sync_setup(cancelled.token).unwrap();
        assert!(app.confirm_sync_setup(cancelled.token).is_err());
        assert!(!app.sync_status().configured);
        let old = app
            .prepare_setup(setup_profile(&app, true), discovery)
            .unwrap();
        let new = app
            .prepare_setup(setup_profile(&app, true), discovery)
            .unwrap();
        assert!(app.confirm_sync_setup(old.token).is_err());
        app.configure_sync(setup_profile(&app, false)).unwrap();
        assert!(app.confirm_sync_setup(new.token).is_err());
        assert_eq!(app.sync_profile().unwrap().issuer, "https://manual.example");
    }

    #[test]
    fn manual_setup_skips_discovery_and_failed_discovery_keeps_saved_profile() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let manual = app
            .prepare_setup(setup_profile(&app, false), |_| {
                panic!("Manual setup must not discover")
            })
            .unwrap();
        assert!(!manual.needs_confirmation);
        app.confirm_sync_setup(manual.token).unwrap();
        let saved = app.service.saved_config().unwrap();
        assert!(app
            .prepare_setup(setup_profile(&app, true), |_| Err(io::Error::other(
                "Discovery failed"
            )))
            .is_err());
        assert!(app.service.saved_config().unwrap() == saved);
        assert!(!app.sync_profile().unwrap().discover_auth);
        assert!(
            app.configure_sync(setup_profile(&app, true)).is_err(),
            "discovery must not bypass review"
        );
    }

    #[test]
    fn changed_issuer_needs_new_confirmation_without_replacing_selection() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let original = app
            .prepare_setup(setup_profile(&app, true), discovery)
            .unwrap();
        app.confirm_sync_setup(original.token).unwrap();
        let saved = app.service.saved_config().unwrap().unwrap();
        let preview = app
            .prepare_setup(setup_profile(&app, true), |mut config| {
                config.issuer = "https://another-identity.example".into();
                Ok(config)
            })
            .unwrap();
        assert!(preview.needs_confirmation);
        assert!(preview.trust_description.contains("Previously accepted"));
        assert!(app.service.saved_config().unwrap().unwrap() == saved);
        app.confirm_sync_setup(preview.token).unwrap();
        assert_eq!(
            app.service.saved_config().unwrap().unwrap().folders,
            saved.folders
        );
    }

    #[test]
    fn discovery_preserves_download_mappings_and_remote_selection() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        app.configure_sync(setup_profile(&app, false)).unwrap();
        let mut saved = app.service.saved_config().unwrap().unwrap();
        let id = "AAAAAAAAAAAAAAAAAAAAAA".to_owned();
        saved
            .folder_mappings
            .insert(id.clone(), app.home.join("remote"));
        saved.selected_folders.insert(id.clone());
        saved.excluded_folders.insert(id);
        saved.auto_download = false;
        saved.download_dir = Some(app.home.join("existing-downloads"));
        app.service.configure(saved.clone()).unwrap();
        let preview = app
            .prepare_setup(setup_profile(&app, true), discovery)
            .unwrap();
        app.confirm_sync_setup(preview.token).unwrap();
        let actual = app.service.saved_config().unwrap().unwrap();
        assert_eq!(actual.folder_mappings, saved.folder_mappings);
        assert_eq!(actual.selected_folders, saved.selected_folders);
        assert_eq!(actual.excluded_folders, saved.excluded_folders);
        assert_eq!(actual.download_dir, saved.download_dir);
        assert!(!actual.auto_download);
    }

    #[test]
    fn passphrase_commands_preserve_explicit_remember_consent() {
        for action in [
            PassphraseAction::Create,
            PassphraseAction::Unlock,
            PassphraseAction::Change,
        ] {
            for remember in [false, true] {
                let command = passphrase_command(
                    action,
                    Zeroizing::new("private passphrase".into()),
                    remember,
                );
                let Command::Passphrase {
                    remember: actual,
                    passphrase,
                    ..
                } = command
                else {
                    panic!("Must use the shared remember-password command")
                };
                assert_eq!(actual, remember);
                assert_eq!(passphrase.as_str(), "private passphrase");
            }
        }
    }

    #[test]
    fn cancellation_signal_does_not_wait_for_the_library_worker() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let cancel = Cancellation::default();
        *app.operation.lock().unwrap() = Some(cancel.clone());
        let _state = app.state().unwrap();
        app.cancel_sync().unwrap();
        assert!(cancel.is_cancelled());
        assert!(!app.sync_status().unlocked);
    }
    #[test]
    fn search_full_contents_and_preserve_metadata() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        app.add_folder("Work".into(), "#abc".into()).unwrap();
        let work = root
            .path()
            .join("Documents/Paper Trail/Work")
            .to_string_lossy()
            .to_string();
        let path = app.create_note(Some(work)).unwrap();
        app.open_note(path.clone()).unwrap();
        app.edit(format!("Title\n{}NEEDLE", "x".repeat(40_000)))
            .unwrap();
        app.save().unwrap();
        app.set_pinned(path.clone(), true).unwrap();
        let snapshot = app.snapshot(None, "needle".into()).unwrap();
        assert_eq!(snapshot.notes.len(), 1);
        assert!(snapshot.notes[0].pinned);
        assert_eq!(snapshot.notes[0].color, "#aabbcc");
        let snapshot = client(root.path()).snapshot(None, "".into()).unwrap();
        assert!(snapshot.notes[0].pinned);
    }
    #[test]
    fn conflict_keeps_edits_and_copy_preserves_external_version() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let path = app.create_note(None).unwrap();
        app.open_note(path.clone()).unwrap();
        app.edit("my edits".into()).unwrap();
        fs::write(&path, "external").unwrap();
        assert!(app.save().is_err());
        let copy = app.save_copy().unwrap();
        assert_eq!(fs::read_to_string(copy).unwrap(), "my edits");
        assert_eq!(fs::read_to_string(path).unwrap(), "external");
    }
    #[test]
    fn rejects_unregistered_paths_and_invalid_names() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let outside = root.path().join("outside.txt");
        fs::write(&outside, "private").unwrap();
        assert!(app.open_note(outside.to_string_lossy().into()).is_err());
        assert!(app.add_folder("../escape".into(), "#fff".into()).is_err());
    }

    #[test]
    fn crud_keeps_txt_extension_and_updates_pins() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let path = app.create_note(None).unwrap();
        app.set_pinned(path.clone(), true).unwrap();
        let path = app.rename_note(path, "renamed".into()).unwrap();
        assert!(path.ends_with("renamed.txt"));
        app.add_folder("Work".into(), "preset-2".into()).unwrap();
        let work = root
            .path()
            .join("Documents/Paper Trail/Work")
            .to_string_lossy()
            .into();
        let path = app.move_note(path, work).unwrap();
        let snapshot = app.snapshot(None, "".into()).unwrap();
        assert_eq!(snapshot.notes.len(), 1);
        assert!(snapshot.notes[0].pinned);
        assert_eq!(snapshot.notes[0].folder_name, "Work");
        app.delete_note(path).unwrap();
        assert!(app.snapshot(None, "".into()).unwrap().notes.is_empty());
    }

    #[test]
    fn all_regular_files_are_visible_but_only_utf8_text_is_editable() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let folder = PathBuf::from(app.snapshot(None, "".into()).unwrap().default_folder);
        for name in ["plain.txt", "README", "journal.md", "UPPER.TXT"] {
            fs::write(folder.join(name), format!("{name}\nSearchable contents")).unwrap();
        }
        let binary = folder.join("image.bin");
        fs::write(&binary, [0, 255, 12]).unwrap();
        let invalid_utf8 = folder.join("invalid");
        fs::write(&invalid_utf8, [255, 254]).unwrap();
        fs::create_dir(folder.join("not-a-note")).unwrap();
        assert_eq!(app.snapshot(None, "".into()).unwrap().notes.len(), 6);
        assert_eq!(
            app.snapshot(None, "searchable".into()).unwrap().notes.len(),
            4
        );
        let markdown = folder.join("journal.md").to_string_lossy().to_string();
        app.open_note(markdown.clone()).unwrap();
        app.edit("Updated Markdown\nPlain UTF-8".into()).unwrap();
        app.close_note().unwrap();
        assert_eq!(
            fs::read_to_string(&markdown).unwrap(),
            "Updated Markdown\nPlain UTF-8"
        );
        assert!(app.open_note(binary.to_string_lossy().into()).is_err());
        assert!(app
            .open_note(invalid_utf8.to_string_lossy().into())
            .is_err());
        assert_eq!(fs::read(binary).unwrap(), [0, 255, 12]);
        assert_eq!(fs::read(invalid_utf8).unwrap(), [255, 254]);
    }

    #[test]
    fn rename_preserves_extensions_contents_and_unpin_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let folder = PathBuf::from(app.snapshot(None, "".into()).unwrap().default_folder);
        for (original, requested, expected) in [
            ("journal.md", "renamed", "renamed.md"),
            ("README", "NOTES", "NOTES"),
            ("upper.TXT", "upper-renamed", "upper-renamed.TXT"),
        ] {
            let path = folder.join(original);
            fs::write(&path, "Keep my contents").unwrap();
            let path = path.to_string_lossy().to_string();
            app.set_pinned(path.clone(), true).unwrap();
            let renamed = app.rename_note(path, requested.into()).unwrap();
            assert_eq!(Path::new(&renamed).file_name().unwrap(), expected);
            assert_eq!(fs::read_to_string(&renamed).unwrap(), "Keep my contents");
            let snapshot = app.snapshot(None, "".into()).unwrap();
            assert!(snapshot.notes.iter().any(|n| n.path == renamed && n.pinned));
            app.set_pinned(renamed, false).unwrap();
        }
        assert!(client(root.path())
            .snapshot(None, "".into())
            .unwrap()
            .notes
            .iter()
            .all(|n| !n.pinned));
    }

    #[test]
    fn rename_does_not_overwrite_a_collision_or_rename_an_open_note() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let path = app.create_note(None).unwrap();
        let existing = Path::new(&path).parent().unwrap().join("existing.txt");
        fs::write(&existing, "Do not overwrite").unwrap();
        app.open_note(path.clone()).unwrap();
        assert!(app
            .rename_note(path.clone(), "existing.txt".into())
            .is_err());
        app.close_note().unwrap();
        let result = app.rename_note(path, "existing.txt".into());
        if let Ok(renamed) = result {
            assert_ne!(Path::new(&renamed), existing);
        }
        assert_eq!(fs::read_to_string(existing).unwrap(), "Do not overwrite");
    }

    #[test]
    fn folder_order_is_validated_and_persisted_without_changing_selection_metadata() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        app.add_folder("Work Notes".into(), "#779b82".into())
            .unwrap();
        let snapshot = app.snapshot(None, "".into()).unwrap();
        let default = snapshot.default_folder;
        let mut paths: Vec<_> = snapshot.folders.iter().map(|f| f.path.clone()).collect();
        paths.reverse();
        app.reorder_folders(paths.clone()).unwrap();
        let snapshot = client(root.path()).snapshot(None, "".into()).unwrap();
        assert_eq!(
            snapshot
                .folders
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>(),
            paths
        );
        assert_eq!(snapshot.default_folder, default);
        let work = snapshot
            .folders
            .iter()
            .find(|f| f.name == "Work Notes")
            .unwrap();
        assert_eq!(work.color, "#779b82");
        assert_eq!(work.badge, colors::badge("Work Notes"));
        assert!(app.reorder_folders(vec![paths[0].clone()]).is_err());
        assert!(app
            .reorder_folders(vec![paths[0].clone(); paths.len()])
            .is_err());
        let mut unknown = paths.clone();
        unknown[0] = root.path().to_string_lossy().into();
        assert!(app.reorder_folders(unknown).is_err());
        assert_eq!(
            app.snapshot(None, "".into())
                .unwrap()
                .folders
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>(),
            paths
        );
    }

    #[test]
    fn folder_edit_changes_only_labels_and_colors_even_when_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let path = app.create_note(None).unwrap();
        app.set_pinned(path.clone(), true).unwrap();
        let before = app.snapshot(None, "".into()).unwrap();
        let folder = before.folders[0].path.clone();
        app.edit_folder(folder.clone(), "  My Notes  ".into(), "#abc".into())
            .unwrap();
        let after = client(root.path()).snapshot(None, "".into()).unwrap();
        assert_eq!(after.default_folder, before.default_folder);
        assert_eq!(after.folders[0].path, folder);
        assert_eq!(after.folders[0].name, "My Notes");
        assert_eq!(after.folders[0].color, "#aabbcc");
        assert_eq!(after.notes[0].path, path);
        assert!(after.notes[0].pinned);
        assert_eq!(after.notes[0].folder_name, "My Notes");
        assert!(Path::new(&folder).is_dir());
        assert!(app
            .edit_folder(folder.clone(), " ".into(), "#fff".into())
            .is_err());
        assert!(app
            .edit_folder(folder.clone(), "Bad\nLabel".into(), "#fff".into())
            .is_err());
        assert!(app
            .edit_folder(
                root.path().to_string_lossy().into(),
                "Outside".into(),
                "#fff".into()
            )
            .is_err());
        let unavailable = PathBuf::from(&folder).with_file_name("temporarily-away");
        fs::rename(&folder, unavailable).unwrap();
        app.edit_folder(folder.clone(), "Offline Notes".into(), "#779b82".into())
            .unwrap();
        let after = app.snapshot(None, "".into()).unwrap();
        assert_eq!(after.folders[0].name, "Offline Notes");
        assert_eq!(after.folders[0].path, folder);
        assert!(!Path::new(&folder).exists());
        assert!(!after.unavailable_folders.is_empty());
    }

    #[test]
    fn moving_preserves_both_files_on_collision_and_keeps_pins() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        app.add_folder("Destination".into(), "#abc".into()).unwrap();
        let snapshot = app.snapshot(None, "".into()).unwrap();
        let source = Path::new(&snapshot.default_folder).join("README");
        let destination = &snapshot
            .folders
            .iter()
            .find(|f| f.name == "Destination")
            .unwrap()
            .path;
        let existing = Path::new(destination).join("README");
        fs::write(&source, "Source contents").unwrap();
        fs::write(&existing, "Destination contents").unwrap();
        let source = source.to_string_lossy().to_string();
        app.set_pinned(source.clone(), true).unwrap();
        app.open_note(source.clone()).unwrap();
        assert!(app.move_note(source.clone(), destination.clone()).is_err());
        app.close_note().unwrap();
        let moved = app.move_note(source.clone(), destination.clone()).unwrap();
        assert!(!Path::new(&source).exists());
        assert_ne!(Path::new(&moved), existing);
        assert_eq!(fs::read_to_string(&moved).unwrap(), "Source contents");
        assert_eq!(
            fs::read_to_string(existing).unwrap(),
            "Destination contents"
        );
        let after = client(root.path()).snapshot(None, "".into()).unwrap();
        assert!(after
            .notes
            .iter()
            .any(|n| n.path == moved && n.pinned && n.folder == *destination));
        assert!(app
            .move_note(moved, root.path().to_string_lossy().into())
            .is_err());
    }

    #[test]
    fn export_preserves_binary_bytes_and_does_not_flush_or_replace_editor_drafts() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let path = app.create_note(None).unwrap();
        fs::write(&path, "Saved version").unwrap();
        app.open_note(path.clone()).unwrap();
        app.edit("Unsaved version".into()).unwrap();
        assert_eq!(app.export_note(path.clone()).unwrap(), b"Saved version");
        assert_eq!(app.state().unwrap().session.text, "Unsaved version");
        let binary = Path::new(&path).parent().unwrap().join("data.bin");
        fs::write(&binary, [0, 255, 254, 13, 10]).unwrap();
        assert_eq!(
            app.export_note(binary.to_string_lossy().into()).unwrap(),
            [0, 255, 254, 13, 10]
        );
        let outside = root.path().join("outside");
        fs::write(&outside, "Private").unwrap();
        assert!(app.export_note(outside.to_string_lossy().into()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_neither_listed_nor_accepted_for_note_operations() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let path = app.create_note(None).unwrap();
        let link = Path::new(&path).parent().unwrap().join("linked.md");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(app.snapshot(None, "".into()).unwrap().notes.len(), 1);
        let link = link.to_string_lossy().to_string();
        assert!(app.open_note(link.clone()).is_err());
        assert!(app.export_note(link.clone()).is_err());
        assert!(app.set_pinned(link.clone(), true).is_err());
        assert!(app.rename_note(link.clone(), "escape".into()).is_err());
        assert!(app.delete_note(link).is_err());
        assert!(Path::new(&path).exists());
    }

    #[test]
    fn container_relocation_rewrites_settings_and_unsynced_profile() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let path = app.create_note(None).unwrap();
        app.set_pinned(path, true).unwrap();
        let default = app.snapshot(None, "".into()).unwrap().default_folder;
        app.configure_sync(SyncProfile {
            server: "https://sync.example".into(),
            issuer: "https://issuer.example".into(),
            client_id: "ios".into(),
            scopes: "openid".into(),
            folders: vec![default],
            discover_auth: false,
        })
        .unwrap();
        drop(app);
        fs::rename(root.path().join("Documents"), root.path().join("Restored")).unwrap();
        let app = PaperTrailClient::new(
            root.path().join("Restored").to_string_lossy().into(),
            root.path().join("Support").to_string_lossy().into(),
        )
        .unwrap();
        let snapshot = app.snapshot(None, "".into()).unwrap();
        assert!(snapshot.notes[0].pinned);
        assert!(snapshot.notes[0].path.contains("/Restored/"));
        assert!(app.sync_profile().unwrap().folders[0].contains("/Restored/"));
        let config = app.service.saved_config().unwrap().unwrap();
        assert!(config
            .download_dir
            .unwrap()
            .to_string_lossy()
            .contains("/Restored/"));
    }

    #[test]
    fn unavailable_folder_is_reported_without_hiding_local_notes() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        app.create_note(None).unwrap();
        app.add_folder("Unavailable".into(), "preset-2".into())
            .unwrap();
        let path = PathBuf::from(
            app.snapshot(None, "".into())
                .unwrap()
                .folders
                .into_iter()
                .find(|folder| folder.name == "Unavailable")
                .unwrap()
                .path,
        );
        fs::remove_dir(&path).unwrap();
        let snapshot = app.snapshot(None, "".into()).unwrap();
        assert_eq!(snapshot.notes.len(), 1);
        assert_eq!(snapshot.unavailable_folders.len(), 1);
        app.forget_folder(path.to_string_lossy().into()).unwrap();
        assert!(app
            .snapshot(None, "".into())
            .unwrap()
            .unavailable_folders
            .is_empty());
    }

    #[test]
    fn default_composition_and_color_changes_use_shared_settings() {
        let root = tempfile::tempdir().unwrap();
        let app = client(root.path());
        let snapshot = app.snapshot(None, "".into()).unwrap();
        assert_eq!(snapshot.appearance, "light");
        assert_eq!(snapshot.folders[0].color, colors::hex("preset-5"));
        app.add_folder("Journal".into(), "#abc".into()).unwrap();
        let folder = root
            .path()
            .join("Documents/Paper Trail/Journal")
            .to_string_lossy()
            .to_string();
        app.register_folder(folder.clone(), true).unwrap();
        app.set_folder_color(folder.clone(), "#def".into()).unwrap();
        let path = app.create_note(None).unwrap();
        assert_eq!(
            Path::new(&path).parent(),
            Some(fs::canonicalize(&folder).unwrap().as_path())
        );
        assert_eq!(
            app.snapshot(None, "".into()).unwrap().notes[0].color,
            "#ddeeff"
        );
    }
}
