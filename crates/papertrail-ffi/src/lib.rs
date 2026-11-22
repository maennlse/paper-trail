// SPDX-License-Identifier: GPL-3.0-or-later
//! Native client boundary. All paths are scoped to explicitly registered folders.
use papertrail_core::{colors, note::sort_notes, Session, Settings, Store};
use papertrail_storage::{Repository, SettingsFile};
use papertrail_sync::{
    auth::LoginFlow,
    control::{Command, Service},
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
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
#[derive(uniffi::Object)]
pub struct PaperTrailClient {
    state: Mutex<State>,
    settings_file: SettingsFile,
    home: PathBuf,
    service: Service,
    login: Mutex<Option<LoginFlow>>,
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
            || target.extension().is_none_or(|ext| ext != "txt")
        {
            return Err(error(
                "Only .txt files inside registered folders are allowed",
            ));
        }
        Ok(target)
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
                if path.extension().is_none_or(|ext| ext != "txt")
                    || !path.is_file()
                    || fs::symlink_metadata(&path)?.file_type().is_symlink()
                {
                    continue;
                }
                let note = match Repository.load(&path) {
                    Ok(note) => note,
                    Err(e) => {
                        unavailable_folders.push(format!("{}: {e}", path.display()));
                        continue;
                    }
                };
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
        if !filename.ends_with(".txt") {
            filename.push_str(".txt");
        }
        let renamed = Repository.rename(&path, &filename)?;
        let mut updated = state.settings.clone();
        updated.rename_note(&path, &renamed.path);
        self.persist(&mut state, updated)?;
        Ok(renamed.path.to_string_lossy().into())
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
        Ok(SyncProfile {
            server: config.server,
            issuer: config.issuer,
            client_id: config.client_id,
            scopes: config.scopes,
            folders: config
                .folders
                .iter()
                .map(|p| p.to_string_lossy().into())
                .collect(),
        })
    }
    pub fn configure_sync(&self, profile: SyncProfile) -> Result<()> {
        let state = self.state()?;
        let folders = profile
            .folders
            .iter()
            .map(|p| self.folder(&state.settings, p))
            .collect::<Result<Vec<_>>>()?;
        let mut config = self.service.saved_config()?.unwrap_or_default();
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
        config.server = profile.server;
        config.issuer = profile.issuer;
        config.client_id = profile.client_id;
        config.scopes = profile.scopes;
        config.folders = folders;
        config
            .download_dir
            .get_or_insert_with(|| self.home.join("Paper Trail Synced"));
        self.service.configure(config)?;
        Ok(())
    }
    pub fn begin_login(&self) -> Result<String> {
        let config = self
            .service
            .saved_config()?
            .ok_or_else(|| error("Configure sync first"))?;
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
    pub fn sync_action(&self, action: SyncAction, secret: String) -> Result<SyncResult> {
        let secret = Zeroizing::new(secret);
        let mut state = self.state()?;
        let command = match action {
            SyncAction::Restore => Command::Restore,
            SyncAction::Status => Command::Status,
            SyncAction::Unlock => Command::Unlock(secret),
            SyncAction::Create => Command::Create(secret),
            SyncAction::Recover => Command::Recover(secret),
            SyncAction::ConfirmRecovery => Command::ConfirmRecovery,
            SyncAction::ChangePassphrase => Command::ChangePassphrase(secret),
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
        let outcome = self
            .service
            .submit(command)
            .recv()
            .map_err(|_| error("Sync worker stopped"))??;
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
        let path = root.path().join("Documents/Paper Trail/Unavailable");
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
        assert!(path.starts_with(&folder));
        assert_eq!(
            app.snapshot(None, "".into()).unwrap().notes[0].color,
            "#ddeeff"
        );
    }
}
