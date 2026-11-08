// SPDX-License-Identifier: GPL-3.0-or-later
//! Frontend-independent worker. Secret prompts remain in the native frontend.
use crate::{
    auth::{self, Tokens},
    engine::Engine,
    error, Api, Config,
};
use papertrail_core::Settings;
use papertrail_crypto::{Secret, Vault};
use papertrail_storage::{atomic_write, SettingsFile};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
};
use zeroize::Zeroizing;

pub enum Command {
    Login,
    DeviceLogin {
        notifications: mpsc::Sender<auth::device::Prompt>,
        cancel: auth::device::Cancellation,
        session_only: bool,
    },
    Unlock(Zeroizing<String>),
    Recover(Zeroizing<String>),
    Create(Zeroizing<String>),
    ChangePassphrase(Zeroizing<String>),
    ConfirmRecovery,
    Lock,
    Logout,
    Sync {
        settings: Box<Settings>,
        protected: Vec<PathBuf>,
    },
    /// GTK keeps editing while HTTP runs; merge metadata on its main thread.
    BackgroundSync {
        settings: Box<Settings>,
    },
    Status,
}
pub struct Outcome {
    pub message: String,
    pub recovery: Option<Zeroizing<String>>,
    pub synced: bool,
    pub unlocked: bool,
    pub settings_update: Option<(Box<Settings>, Box<Settings>)>,
    pub local_changed: bool,
}
struct PendingVault {
    vault: Vault,
    key: Secret,
    expected: u64,
}
struct Control {
    local: crate::local::LocalAccess,
    root: PathBuf,
    settings_path: PathBuf,
    config: Option<Config>,
    tokens: Option<Tokens>,
    persist_tokens: bool,
    memory_only_tokens: bool,
    key: Option<Secret>,
    pending: Option<PendingVault>,
}
#[derive(Clone)]
pub struct Service {
    control: Arc<Mutex<Control>>,
    root: Arc<PathBuf>,
    local: crate::local::LocalAccess,
}
impl Service {
    pub fn saved_config(&self) -> io::Result<Option<Config>> {
        match std::fs::read(self.root.join("config.json")) {
            Ok(bytes) => crate::read_config(&bytes).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn folder_choices(
        &self,
        settings: &Settings,
    ) -> io::Result<Vec<crate::folders::FolderChoice>> {
        let config = self.saved_config()?.unwrap_or_default();
        let state = self.snapshot()?;
        let mut choices = Vec::new();
        if let Some(state) = &state {
            if let Some(catalog) = state.catalog.as_ref().or(state.catalog_draft.as_ref()) {
                for (id, data) in &catalog.folders {
                    let path = config
                        .folder_mappings
                        .get(id)
                        .cloned()
                        .or_else(|| state.bindings.get(id).map(|b| b.path.clone()));
                    let selected = !config.excluded_folders.contains(id)
                        && path
                            .as_ref()
                            .map(|p| {
                                config.folders.contains(p)
                                    || config.folder_mappings.get(id) == Some(p)
                                    || config.selected_folders.contains(id)
                                    || (config.auto_download
                                        && state.bindings.get(id).is_some_and(|b| b.managed))
                            })
                            .unwrap_or(
                                config.auto_download || config.selected_folders.contains(id),
                            );
                    choices.push(crate::folders::FolderChoice {
                        id: Some(id.clone()),
                        path,
                        name: data.name.clone(),
                        selected,
                        conflict: state.folder_conflicts.contains(id),
                    });
                }
            }
        }
        for path in settings.note_folders.iter().chain(&config.folders) {
            if !choices
                .iter()
                .any(|choice| choice.path.as_ref() == Some(path))
            {
                choices.push(crate::folders::FolderChoice {
                    id: None,
                    path: Some(path.clone()),
                    name: settings.folder_name(path),
                    selected: config.folders.contains(path),
                    conflict: false,
                });
            }
        }
        Ok(choices)
    }
    pub fn snapshot(&self) -> io::Result<Option<crate::engine::State>> {
        match std::fs::read(self.root.join("state.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| error("Invalid sync journal")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn accept_remote_folder(&self, id: &str, settings: &Settings) -> io::Result<()> {
        let _control = self
            .control
            .try_lock()
            .map_err(|_| error("Wait for the current sync before resolving folder metadata"))?;
        let mut engine = Engine::open(self.root.join("state.json"))?;
        if !engine
            .state
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.folders.contains_key(id))
        {
            return Err(error("Unknown remote folder ID"));
        }
        if let Some(b) = engine.state.bindings.get_mut(id) {
            b.observed_name = b
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into();
            b.observed_color = settings.folder_colors.get(&b.path).cloned();
        }
        engine.state.folder_conflicts.remove(id);
        engine.save()
    }
    pub fn install_tokens(&self, tokens: Tokens) -> io::Result<()> {
        let mut c = self
            .control
            .lock()
            .map_err(|_| error("Sync worker unavailable"))?;
        let config = c.config()?;
        let api = Api::new(&config, tokens.access.clone())?;
        let account = api.identity()?;
        if account.iss != config.issuer {
            return Err(error("API account issuer mismatch"));
        }
        let engine = Engine::open(c.root.join("state.json"))?;
        if engine
            .state
            .account
            .as_ref()
            .is_some_and(|old| old != &account)
        {
            return Err(error("Different account; use a separate local profile"));
        }
        c.persist_tokens = !c.memory_only_tokens && auth::save(&config, &tokens).is_ok();
        c.tokens = Some(tokens);
        Ok(())
    }
    pub fn relocate(&self, old: &Path, new: &Path) -> io::Result<()> {
        if !self.root.join("state.json").exists() {
            return Ok(());
        }
        crate::local::relocations(&self.root, |entries| {
            entries.push(crate::local::Relocation {
                id: papertrail_crypto::opaque_id(),
                old: old.into(),
                new: new.into(),
            });
            Ok(())
        })?;
        if let Some(mut config) = self.saved_config()? {
            let rewrite = |path: &PathBuf| {
                path.strip_prefix(old)
                    .map(|suffix| {
                        if suffix.as_os_str().is_empty() {
                            new.into()
                        } else {
                            new.join(suffix)
                        }
                    })
                    .unwrap_or_else(|_| path.clone())
            };
            config.folders = config.folders.iter().map(rewrite).collect();
            config.folder_mappings = config
                .folder_mappings
                .iter()
                .map(|(id, p)| (id.clone(), rewrite(p)))
                .collect();
            atomic_write(
                &self.root.join("config.json"),
                &serde_json::to_vec_pretty(&config).map_err(error)?,
            )?;
        }
        Ok(())
    }
    pub fn local(&self) -> io::Result<std::sync::MutexGuard<'_, crate::local::Activity>> {
        self.local
            .lock()
            .map_err(|_| error("Local note access unavailable"))
    }
    pub fn new(settings_path: &Path) -> Self {
        Self::with_storage_mode(settings_path, false)
    }
    /// Headless clients must not depend on a desktop credential-store prompt.
    pub fn new_session_only(settings_path: &Path) -> Self {
        Self::with_storage_mode(settings_path, true)
    }
    fn with_storage_mode(settings_path: &Path, memory_only_tokens: bool) -> Self {
        let root = settings_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("sync");
        let local = crate::local::LocalAccess::default();
        Self {
            root: Arc::new(root.clone()),
            local: local.clone(),
            control: Arc::new(Mutex::new(Control {
                local,
                root,
                settings_path: settings_path.into(),
                config: None,
                tokens: None,
                persist_tokens: false,
                memory_only_tokens,
                key: None,
                pending: None,
            })),
        }
    }
    pub fn configured(&self) -> bool {
        self.root.join("config.json").is_file()
    }
    pub fn configure(&self, mut config: Config) -> io::Result<()> {
        config.validate()?;
        config.version = 2;
        let mut c = self
            .control
            .try_lock()
            .map_err(|_| error("Sync worker unavailable"))?;
        std::fs::create_dir_all(&c.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&c.root, std::fs::Permissions::from_mode(0o700))?;
        }
        if c.root.join("state.json").exists() {
            let old: Config = serde_json::from_slice(&std::fs::read(c.root.join("config.json"))?)
                .map_err(error)?;
            if old.server != config.server
                || old.issuer != config.issuer
                || old.client_id != config.client_id
            {
                return Err(error("Use a separate settings profile to change the server/account after vault setup"));
            }
            let activity = self.local()?;
            if let Some(state) = self.snapshot()? {
                if let Some(catalog) = state.catalog.as_ref().or(state.catalog_draft.as_ref()) {
                    if config
                        .folder_mappings
                        .keys()
                        .any(|id| !catalog.folders.contains_key(id))
                    {
                        return Err(error(
                            "Unknown remote folder ID in local mapping; use the listed folder IDs",
                        ));
                    }
                }
                for (id, path) in &config.folder_mappings {
                    if let Some(old) = state.bindings.get(id).filter(|b| b.path != *path) {
                        if activity.protected.iter().any(|p| p.starts_with(&old.path)) {
                            return Err(error(
                                "Close notes in a folder before changing its local mapping",
                            ));
                        }
                    }
                }
            }
        }
        atomic_write(
            &c.root.join("config.json"),
            &serde_json::to_vec_pretty(&config).map_err(error)?,
        )?;
        c.config = Some(config);
        Ok(())
    }
    pub fn submit(&self, command: Command) -> Receiver<io::Result<Outcome>> {
        let (tx, rx) = mpsc::channel();
        let service = self.clone();
        std::thread::spawn(move || {
            let result = service
                .control
                .lock()
                .map_err(|_| error("Sync worker unavailable"))
                .and_then(|mut c| c.execute(command));
            let _ = tx.send(result);
        });
        rx
    }
}
impl Control {
    fn config(&mut self) -> io::Result<Config> {
        self.config = Some(crate::read_config(
            &std::fs::read(self.root.join("config.json"))
                .map_err(|_| error("Configure Sync first"))?,
        )?);
        let config = self.config.as_ref().unwrap().clone();
        config.validate()?;
        Ok(config)
    }
    fn execute(&mut self, command: Command) -> io::Result<Outcome> {
        let background = matches!(&command, Command::BackgroundSync { .. });
        let command = match command {
            Command::BackgroundSync { settings } => Command::Sync {
                settings,
                protected: Vec::new(),
            },
            command => command,
        };
        let mut outcome = Outcome {
            message: String::new(),
            recovery: None,
            synced: false,
            unlocked: self.key.is_some(),
            settings_update: None,
            local_changed: false,
        };
        if matches!(command, Command::Lock) {
            self.key = None;
            self.pending = None;
            outcome.unlocked = false;
            outcome.message = "Vault locked; local plaintext files remain accessible".into();
            return Ok(outcome);
        }
        if matches!(command, Command::Logout) {
            self.key = None;
            self.pending = None;
            self.tokens = None;
            outcome.unlocked = false;
            self.persist_tokens = false;
            if !self.memory_only_tokens {
                let config = self.config().or_else(|e| self.config.clone().ok_or(e))?;
                auth::logout(&config)?;
            }
            outcome.message = "Signed out; local notes and rollback checkpoints retained".into();
            return Ok(outcome);
        }
        let config = self.config()?;
        if matches!(command, Command::Status) {
            outcome.message = format!(
                "Server: {} · {} · {}",
                config.server,
                if self.tokens.is_some() {
                    "signed in"
                } else {
                    "sign in required"
                },
                if self.key.is_some() {
                    "vault unlocked"
                } else {
                    "vault locked"
                }
            );
            return Ok(outcome);
        }
        if matches!(&command, Command::Login | Command::DeviceLogin { .. }) {
            let (tokens, cancel, session_only) = match command {
                Command::Login => (auth::login(&config)?, None, false),
                Command::DeviceLogin {
                    notifications,
                    cancel,
                    session_only,
                } => {
                    let tokens = auth::device::login(
                        &config,
                        |prompt| {
                            notifications.send(prompt).map_err(|_| {
                                io::Error::new(
                                    io::ErrorKind::Interrupted,
                                    "Device sign-in display closed",
                                )
                            })
                        },
                        &cancel,
                    )?;
                    (tokens, Some(cancel), session_only)
                }
                _ => unreachable!(),
            };
            let api = Api::new(&config, tokens.access.clone())?;
            let account = api.identity()?;
            if account.iss != config.issuer {
                return Err(error("API account issuer mismatch"));
            }
            // Never overwrite another account's local working tree.
            let engine = Engine::open(self.root.join("state.json"))?;
            if engine
                .state
                .account
                .as_ref()
                .is_some_and(|old| old != &account)
            {
                return Err(error(
                    "Different account: use a separate settings profile and local note folder",
                ));
            }
            // Closing cancellation before touching secure storage avoids a late
            // cancel overwriting/deleting a previously stored session. The UI
            // reports "already approved" if cancellation is now too late.
            if cancel.as_ref().is_some_and(|cancel| !cancel.commit()) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Device sign-in cancelled",
                ));
            }
            let session_only = session_only || self.memory_only_tokens;
            self.memory_only_tokens = session_only;
            let saved = !session_only && auth::save(&config, &tokens).is_ok();
            self.persist_tokens = saved;
            self.tokens = Some(tokens);
            outcome.message = if session_only {
                "Signed in for this SSH/headless session only. Unlock or create your vault."
            } else if saved {
                "Signed in. Unlock or create your vault."
            } else {
                "Signed in for this session only: secure credential storage unavailable."
            }
            .into();
            return Ok(outcome);
        }
        if self.tokens.is_none() {
            if self.memory_only_tokens {
                return Err(error(
                    "Sign in for this SSH/headless session with :login before unlocking or syncing",
                ));
            }
            self.tokens = Some(auth::load(&config)?);
            self.persist_tokens = true;
        }
        let api = Api::new(
            &config,
            auth::access_with_persistence(
                &config,
                self.tokens.as_mut().unwrap(),
                self.persist_tokens,
            )?,
        )?;
        let account = api.identity()?;
        if account.iss != config.issuer {
            return Err(error("API account issuer mismatch"));
        }
        let mut engine = Engine::open(self.root.join("state.json"))?;
        engine.local = self.local.clone();
        let vault = api.vault()?;
        if let Some(v) = &vault {
            if engine
                .state
                .account
                .as_ref()
                .is_some_and(|old| old != &account)
                || (!engine.state.vault_id.is_empty() && engine.state.vault_id != v.id)
                || v.generation < engine.state.generation
            {
                return Err(error(
                    "Account/vault changed or server envelope rolled back; sync stopped",
                ));
            }
        } else if !engine.state.vault_id.is_empty() {
            return Err(error(
                "Server lost an existing vault; refusing to create a replacement",
            ));
        }
        match command {
            Command::Create(passphrase) => {
                if vault.is_some() {
                    return Err(error("A vault already exists; unlock it"));
                }
                strong(&passphrase)?;
                let (vault, key, recovery) = Vault::create(&account, &passphrase).map_err(error)?;
                self.pending = Some(PendingVault {
                    vault,
                    key,
                    expected: 0,
                });
                outcome.recovery = Some(recovery.encode());
                outcome.message = "Save this recovery key now. Confirm only after saving it; the vault is not created until confirmation.".into();
            }
            Command::ChangePassphrase(passphrase) => {
                strong(&passphrase)?;
                let vault = vault.ok_or_else(|| error("Create a vault first"))?;
                let key = self
                    .key
                    .as_ref()
                    .ok_or_else(|| error("Unlock the vault first"))?;
                let (new, recovery) = vault.rewrap(&account, &passphrase, key).map_err(error)?;
                self.pending = Some(PendingVault {
                    vault: new,
                    key: Secret(key.0),
                    expected: vault.generation,
                });
                outcome.recovery = Some(recovery.encode());
                outcome.message = "Save the NEW recovery key. Confirmation replaces both wrapping credentials without re-encrypting notes.".into();
            }
            Command::ConfirmRecovery => {
                let pending = self
                    .pending
                    .as_ref()
                    .ok_or_else(|| error("No recovery confirmation pending"))?;
                api.put_vault(&pending.vault, pending.expected)?;
                engine.bind(&account, &pending.vault)?;
                self.key = Some(Secret(pending.key.0));
                self.pending = None;
                outcome.message = "Recovery key confirmed; vault unlocked".into();
            }
            Command::Unlock(passphrase) => {
                let vault = vault.ok_or_else(|| error("Create a vault first"))?;
                let key = vault.unlock(&account, &passphrase).map_err(error)?;
                engine.bind(&account, &vault)?;
                self.key = Some(key);
                outcome.message = "Vault unlocked for this session".into();
            }
            Command::Recover(recovery) => {
                let vault = vault.ok_or_else(|| error("No vault to recover"))?;
                let key = vault
                    .recover(&account, &Secret::decode(&recovery).map_err(error)?)
                    .map_err(error)?;
                engine.bind(&account, &vault)?;
                self.key = Some(key);
                outcome.message = "Vault recovered. You can now change its passphrase.".into();
            }
            Command::Sync {
                settings,
                protected,
            } => {
                let vault = vault.ok_or_else(|| error("Create a vault first"))?;
                let key = self
                    .key
                    .as_ref()
                    .ok_or_else(|| error("Unlock the vault to sync"))?;
                engine.bind(&account, &vault)?;
                outcome.message = engine.run(&api, &config, key, &settings, &protected)?;
                outcome.local_changed = engine.local_changed;
                // Sync metadata back into the existing path-keyed settings model.
                let file = SettingsFile {
                    path: self.settings_path.clone(),
                };
                if file.path.exists() {
                    let mut updated = (*settings).clone();
                    engine.folder_settings(&config, &mut updated);
                    for t in engine.state.notes.values() {
                        if t.deleted {
                            updated.delete_note(&t.path);
                        } else if let Some(data) = &t.baseline {
                            updated.set_pinned(&t.path, data.pinned);
                            if let Some(language) = &data.language {
                                updated
                                    .note_language_overrides
                                    .insert(t.path.clone(), language.clone());
                            } else {
                                updated.note_language_overrides.remove(&t.path);
                            }
                        }
                    }
                    if background {
                        outcome.settings_update = Some((settings, Box::new(updated)));
                    } else {
                        let home = std::env::var_os("HOME")
                            .map(PathBuf::from)
                            .unwrap_or_default();
                        file.save_merged(&updated, &settings, &home)?;
                    }
                }
                outcome.synced = true;
            }
            _ => unreachable!(),
        }
        outcome.unlocked = self.key.is_some();
        Ok(outcome)
    }
}
fn strong(passphrase: &str) -> io::Result<()> {
    if passphrase.chars().count() < 16 {
        return Err(error("Use a strong vault passphrase of at least 16 characters, preferably several random words"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_operations_do_not_wait_for_the_network_controller_mutex() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new(&temp.path().join("settings.json"));
        std::fs::create_dir_all(&*service.root).unwrap();
        std::fs::write(service.root.join("state.json"), "{}").unwrap();
        let controller = service.control.lock().unwrap();
        let worker = service.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _local = worker.local().unwrap();
            worker
                .relocate(Path::new("old.txt"), Path::new("new.txt"))
                .unwrap();
            tx.send(()).unwrap();
        });
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .expect("local operation waited for network mutex");
        drop(controller);
        thread.join().unwrap();
    }

    #[test]
    fn headless_sessions_do_not_load_save_or_delete_desktop_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        assert!(
            Service::new_session_only(&path)
                .submit(Command::Logout)
                .recv()
                .unwrap()
                .is_ok(),
            "headless logout should not require config or keyring access"
        );
        let service = Service::new_session_only(&path);
        service
            .configure(Config {
                version: 2,
                server: "https://sync.example".into(),
                issuer: "https://issuer.example".into(),
                client_id: "public-native".into(),
                scopes: "openid offline_access".into(),
                folders: vec![temp.path().join("notes")],
                ..Config::default()
            })
            .unwrap();
        let err = service
            .submit(Command::Unlock(Zeroizing::new("not sent anywhere".into())))
            .recv()
            .unwrap()
            .err()
            .unwrap();
        assert!(err.to_string().contains(":login"));
        {
            let mut control = service.control.lock().unwrap();
            assert!(control.memory_only_tokens);
            assert!(!control.persist_tokens);
            control.tokens = Some(Tokens {
                access: "session-only-access".into(),
                refresh: Some("session-only-refresh".into()),
                expires: u64::MAX,
            });
        }
        let result = service.submit(Command::Logout).recv().unwrap().unwrap();
        assert!(!result.unlocked);
        let control = service.control.lock().unwrap();
        assert!(control.tokens.is_none());
        assert!(!control.persist_tokens);
        assert!(control.memory_only_tokens);
        assert!(!std::fs::read_to_string(service.root.join("config.json"))
            .unwrap()
            .contains("session-only"));
        drop(control);
        assert!(
            !Service::new(&path)
                .control
                .lock()
                .unwrap()
                .memory_only_tokens
        );
    }
}
