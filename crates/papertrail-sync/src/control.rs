// SPDX-License-Identifier: GPL-3.0-or-later
//! Frontend-independent worker. Secret prompts remain in the native frontend.
pub use crate::auth::device::Cancellation;
use crate::{
    auth::{self, Tokens},
    credentials::{self, Remembered},
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
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
};
use zeroize::Zeroizing;

fn check_cancelled(cancel: &Cancellation) -> io::Result<()> {
    if cancel.is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Sync operation cancelled",
        ))
    } else {
        Ok(())
    }
}

struct CancellableRemote<'a> {
    api: &'a Api,
    cancel: &'a Cancellation,
}
impl crate::engine::Remote for CancellableRemote<'_> {
    fn check_cancelled(&self) -> io::Result<()> {
        check_cancelled(self.cancel)
    }
    fn changes(&self, cursor: u64, through: Option<u64>) -> io::Result<papertrail_crypto::Changes> {
        check_cancelled(self.cancel)?;
        self.api.changes(cursor, through)
    }
    fn upload(&self, revision: &papertrail_crypto::Revision) -> io::Result<Option<u64>> {
        check_cancelled(self.cancel)?;
        // Always return an in-flight receipt so the engine journals it before stopping.
        self.api.upload(revision)
    }
    fn upload_checked(
        &self,
        revision: &papertrail_crypto::Revision,
        head: Option<&str>,
    ) -> io::Result<Option<u64>> {
        check_cancelled(self.cancel)?;
        self.api.upload_checked(revision, head)
    }
    fn erase_folder(&self, request: &papertrail_crypto::Erasure) -> io::Result<()> {
        check_cancelled(self.cancel)?;
        self.api.erase_folder(request)
    }
}

pub enum Command {
    /// Restore login and an explicitly remembered vault password at startup.
    Restore,
    Passphrase {
        action: PassphraseAction,
        passphrase: Zeroizing<String>,
        remember: bool,
    },
    Login,
    BrowserLogin {
        events: mpsc::Sender<auth::BrowserEvent>,
    },
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
    DeleteFolder {
        id: String,
        settings: Box<Settings>,
    },
}
#[derive(Clone, Copy)]
pub enum PassphraseAction {
    Create,
    Unlock,
    Change,
}
pub struct Outcome {
    pub message: String,
    pub warning: Option<String>,
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
    passphrase: Option<Zeroizing<String>>,
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
    secrets: Box<dyn credentials::Store>,
    auto_unlock_blocked: bool,
    known_vault: Option<(papertrail_crypto::Account, bool)>,
}
#[derive(Clone)]
pub struct Service {
    control: Arc<Mutex<Control>>,
    signed_in: Arc<AtomicBool>,
    unlocked: Arc<AtomicBool>,
    vault_state: Arc<AtomicU8>,
    root: Arc<PathBuf>,
    local: crate::local::LocalAccess,
}
impl Service {
    /// Stop this device's sync without touching note files, history, or pending work.
    /// Does not take the controller/journal lock: closing must work during HTTP.
    pub fn pause_folder(&self, path: &Path) -> io::Result<()> {
        let _activity = self.local()?;
        let Some(mut config) = self.saved_config()? else {
            return Ok(());
        };
        let download_dir = config.download_root()?;
        let mut ids = std::collections::BTreeSet::new();
        for (id, mapped) in &config.folder_mappings {
            if crate::same_directory(mapped, path) {
                ids.insert(id.clone());
            }
        }
        if let Some(state) = self.snapshot()? {
            crate::local::relocations(&self.root, |entries| {
                for (id, binding) in &state.bindings {
                    let mut mapped = binding.path.clone();
                    for entry in entries
                        .iter()
                        .filter(|entry| !state.relocations.contains(&entry.id))
                    {
                        if let Ok(suffix) = mapped.strip_prefix(&entry.old) {
                            mapped = entry.new.join(suffix);
                        }
                    }
                    if crate::same_directory(&mapped, path) {
                        ids.insert(id.clone());
                    }
                }
                Ok(())
            })?;
        }
        config
            .folders
            .retain(|folder| !crate::same_directory(folder, path));
        config.selected_folders.retain(|id| !ids.contains(id));
        config.excluded_folders.extend(ids);
        // Closing the last selected upload folder must leave a valid profile.
        config.download_dir = Some(download_dir);
        config.validate()?;
        atomic_write(
            &self.root.join("config.json"),
            &serde_json::to_vec_pretty(&config).map_err(error)?,
        )
    }

    /// Explicit opt-in to publish a retained local folder under a fresh identity.
    /// This is never called by background reconciliation.
    pub fn sync_as_new_folder(&self, id: &str) -> io::Result<()> {
        let mut control = self
            .control
            .try_lock()
            .map_err(|_| error("Wait for the current sync"))?;
        let mut config = control.config()?;
        let mut engine = Engine::open(self.root.join("state.json"))?;
        let _activity = self.local()?;
        engine.relocate_pending()?;
        let mut catalog = engine
            .state
            .catalog
            .clone()
            .ok_or_else(|| error("No synced folders"))?;
        if !catalog.deleted_folders.contains(id) {
            return Err(error("This folder has not been removed from sync"));
        }
        let binding = engine
            .state
            .bindings
            .get(id)
            .cloned()
            .ok_or_else(|| error("No retained local folder"))?;
        if !binding.path.is_dir() {
            return Err(error("Local folder is unavailable"));
        }
        let data = catalog
            .folders
            .get(id)
            .cloned()
            .ok_or_else(|| error("Unknown folder"))?;
        let new_id = papertrail_crypto::opaque_id();
        let removed: std::collections::BTreeSet<_> = engine
            .state
            .notes
            .iter()
            .filter(|(_, t)| t.path.parent() == Some(binding.path.as_path()))
            .map(|(id, _)| id.clone())
            .collect();
        engine.state.notes.retain(|id, _| !removed.contains(id));
        engine
            .state
            .outgoing
            .retain(|r| !removed.contains(&r.note_id));
        engine
            .state
            .deferred
            .retain(|r| !removed.contains(&r.revision.note_id));
        if engine
            .state
            .applying
            .as_ref()
            .is_some_and(|pending| pending.path.parent() == Some(binding.path.as_path()))
        {
            engine.state.applying = None;
        }
        engine.state.bindings.remove(id);
        engine
            .state
            .bindings
            .insert(new_id.clone(), binding.clone());
        catalog.folders.insert(new_id.clone(), data);
        engine.state.catalog_draft = Some(catalog);
        config.folder_mappings.remove(id);
        config.selected_folders.remove(id);
        config.excluded_folders.insert(id.into());
        config.folder_mappings.insert(new_id.clone(), binding.path);
        config.selected_folders.insert(new_id);
        engine.save()?;
        atomic_write(
            &self.root.join("config.json"),
            &serde_json::to_vec_pretty(&config).map_err(error)?,
        )?;
        control.config = Some(config);
        Ok(())
    }

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
            // A stale unpublished draft must never hide newly discovered IDs.
            let mut visible_catalog = state.catalog.clone().or(state.catalog_draft.clone());
            if let (Some(visible), Some(draft)) = (&mut visible_catalog, &state.catalog_draft) {
                for (id, data) in &draft.folders {
                    visible
                        .folders
                        .entry(id.clone())
                        .or_insert_with(|| data.clone());
                }
                visible
                    .deleted_folders
                    .extend(draft.deleted_folders.iter().cloned());
            }
            if let Some(catalog) = &visible_catalog {
                for (id, data) in &catalog.folders {
                    if catalog.deleted_folders.contains(id) {
                        continue;
                    }
                    let path = config
                        .folder_mappings
                        .get(id)
                        .cloned()
                        .or_else(|| state.bindings.get(id).map(|b| b.path.clone()));
                    let published = state
                        .catalog
                        .as_ref()
                        .is_some_and(|c| c.folders.contains_key(id));
                    if !published
                        && !path.as_ref().is_some_and(|p| {
                            settings
                                .note_folders
                                .iter()
                                .any(|open| crate::same_directory(open, p))
                        })
                    {
                        continue;
                    }
                    let selected = !config.excluded_folders.contains(id)
                        && path
                            .as_ref()
                            .map(|p| {
                                config
                                    .folders
                                    .iter()
                                    .any(|folder| crate::same_directory(folder, p))
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
                        stored_path: state.bindings.get(id).and_then(|b| {
                            config
                                .folder_mappings
                                .get(id)
                                .filter(|path| !crate::same_directory(path, &b.path))
                                .map(|_| b.path.clone())
                        }),
                    });
                }
            }
        }
        for path in &settings.note_folders {
            if state.as_ref().is_some_and(|s| {
                s.bindings.iter().any(|(id, b)| {
                    crate::same_directory(&b.path, path)
                        && s.catalog
                            .as_ref()
                            .is_some_and(|c| c.deleted_folders.contains(id))
                })
            }) {
                continue;
            }
            if !choices.iter().any(|choice| {
                choice
                    .path
                    .as_ref()
                    .is_some_and(|p| crate::same_directory(p, path))
            }) {
                choices.push(crate::folders::FolderChoice {
                    id: None,
                    path: Some(path.clone()),
                    name: settings.folder_name(path),
                    selected: config
                        .folders
                        .iter()
                        .any(|folder| crate::same_directory(folder, path)),
                    conflict: false,
                    stored_path: None,
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
            b.observed_label = settings.folder_labels.get(&b.path).cloned();
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
        auth::config::check(&config)?;
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
        c.observe_bound_vault(&engine, &account);
        // Match browser login: a failed vault lookup is unknown, not absence.
        let _ = c.observe_vault(&account, api.vault());
        c.persist_tokens = !c.memory_only_tokens && auth::save(&config, &tokens).is_ok();
        c.tokens = Some(tokens);
        self.publish_session_state(&c);
        Ok(())
    }
    pub fn relocate(&self, old: &Path, new: &Path) -> io::Result<()> {
        if self.root.join("state.json").exists() {
            crate::local::relocations(&self.root, |entries| {
                entries.push(crate::local::Relocation {
                    id: papertrail_crypto::opaque_id(),
                    old: old.into(),
                    new: new.into(),
                });
                Ok(())
            })?;
        }
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
            config.download_dir = config.download_dir.as_ref().map(rewrite);
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
            signed_in: Arc::new(AtomicBool::new(false)),
            unlocked: Arc::new(AtomicBool::new(false)),
            vault_state: Arc::new(AtomicU8::new(0)),
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
                secrets: Box::new(credentials::Platform),
                auto_unlock_blocked: false,
                known_vault: None,
            })),
        }
    }
    pub fn configured(&self) -> bool {
        self.root.join("config.json").is_file()
    }
    /// Last completed worker/configuration operation's session state. Reading
    /// this never waits for the network worker or accesses the credential store.
    pub fn signed_in(&self) -> bool {
        self.signed_in.load(Ordering::Acquire)
    }
    /// Last completed operation's key state, including operations returning errors.
    pub fn unlocked(&self) -> bool {
        self.unlocked.load(Ordering::Acquire)
    }
    /// Last confirmed account's vault state: unknown is not the same as absent.
    /// Reading this never waits for a network or credential-store operation.
    pub fn vault_exists(&self) -> Option<bool> {
        match self.vault_state.load(Ordering::Acquire) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }
    fn publish_session_state(&self, control: &Control) {
        self.unlocked
            .store(control.key.is_some(), Ordering::Release);
        self.signed_in
            .store(control.tokens.is_some(), Ordering::Release);
        self.vault_state.store(
            match control.known_vault.as_ref().map(|(_, exists)| *exists) {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            },
            Ordering::Release,
        );
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
        let previous = match std::fs::read(c.root.join("config.json")) {
            Ok(bytes) => Some(crate::read_config(&bytes)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if c.root.join("state.json").exists() {
            let old = previous.as_ref().ok_or_else(|| {
                error("Existing journal has no sync configuration; restore it before continuing")
            })?;
            if old.server != config.server
                || old.issuer != config.issuer
                || old.client_id != config.client_id
            {
                return Err(error("Use a separate settings profile to change the server/account after vault setup"));
            }
            let activity = self.local()?;
            if let Some(state) = self.snapshot()? {
                crate::folders::validate_folder_mappings(&state, &config, &activity.protected)?;
                if let Some(catalog) = state.catalog.as_ref().or(state.catalog_draft.as_ref()) {
                    if config.folder_mappings.keys().any(|id| {
                        !catalog.folders.contains_key(id)
                            && !state
                                .catalog_draft
                                .as_ref()
                                .is_some_and(|draft| draft.folders.contains_key(id))
                    }) {
                        return Err(error(
                            "Unknown remote folder ID in local mapping; use the listed folder IDs",
                        ));
                    }
                }
            }
        }
        atomic_write(
            &c.root.join("config.json"),
            &serde_json::to_vec_pretty(&config).map_err(error)?,
        )?;
        if previous.as_ref().is_some_and(|old| {
            old.server != config.server
                || old.issuer != config.issuer
                || old.client_id != config.client_id
        }) {
            // Never carry an old identity's in-memory credentials or keys into
            // an explicitly reconfigured, still-unbound profile.
            c.clear_identity();
        }
        c.config = Some(config);
        self.publish_session_state(&c);
        Ok(())
    }
    pub fn submit(&self, command: Command) -> Receiver<io::Result<Outcome>> {
        self.submit_cancellable(command, Cancellation::default())
    }
    /// Stop at safe checkpoints; a blocking HTTP request retains its normal timeout.
    pub fn submit_cancellable(
        &self,
        command: Command,
        cancel: Cancellation,
    ) -> Receiver<io::Result<Outcome>> {
        let (tx, rx) = mpsc::channel();
        let service = self.clone();
        std::thread::spawn(move || {
            let result = service
                .control
                .lock()
                .map_err(|_| error("Sync worker unavailable"))
                .and_then(|mut c| {
                    let result = c.execute_cancellable(command, &cancel);
                    // Errors may still have restored a session, or completed
                    // local sign-out before a credential-store removal failed.
                    service.publish_session_state(&c);
                    result
                });
            let _ = tx.send(result);
        });
        rx
    }
}
impl Control {
    fn observe_vault(
        &mut self,
        account: &papertrail_crypto::Account,
        result: io::Result<Option<Vault>>,
    ) -> io::Result<Option<Vault>> {
        if self
            .known_vault
            .as_ref()
            .is_some_and(|(old, _)| old != account)
        {
            self.known_vault = None;
        }
        let vault = result?;
        // Vault deletion is not supported. A server losing a known vault must
        // never make the frontend offer creation of a replacement.
        let existed = self.known_vault.as_ref().is_some_and(|(_, exists)| *exists);
        self.known_vault = Some((account.clone(), existed || vault.is_some()));
        Ok(vault)
    }
    fn observe_bound_vault(&mut self, engine: &Engine, account: &papertrail_crypto::Account) {
        if engine.state.account.as_ref() == Some(account) && !engine.state.vault_id.is_empty() {
            self.known_vault = Some((account.clone(), true));
        }
    }
    fn forget_credentials(
        &self,
        config: &Config,
        delete_login: impl FnOnce(&Config) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.memory_only_tokens {
            return Ok(());
        }
        // Attempt both removals even if one fails.
        let password = self.secrets.delete(config);
        let login = delete_login(config);
        match (password, login) {
            (Err(password), Err(login)) => Err(error(format!("{password}; {login}"))),
            (password, login) => password.and(login),
        }
    }
    fn restore_vault(
        &mut self,
        config: &Config,
        account: &papertrail_crypto::Account,
        vault: &Vault,
        engine: &mut Engine,
    ) -> io::Result<bool> {
        if self.memory_only_tokens || self.auto_unlock_blocked {
            return Ok(false);
        }
        let Some(saved) = self.secrets.load(config)? else {
            return Ok(false);
        };
        let passphrase = Remembered::decode(&saved, account, vault)?;
        self.unlock_vault(account, vault, engine, &passphrase, None)?;
        Ok(true)
    }
    fn remember_password(
        &self,
        config: &Config,
        account: &papertrail_crypto::Account,
        vault: &Vault,
        passphrase: Option<&str>,
    ) -> io::Result<()> {
        if self.memory_only_tokens {
            return Ok(());
        }
        match passphrase {
            Some(passphrase) => Remembered::encode(account, vault, passphrase)
                .and_then(|value| self.secrets.save(config, &value)),
            None => self.secrets.delete(config),
        }
    }
    fn unlock_vault(
        &mut self,
        account: &papertrail_crypto::Account,
        vault: &Vault,
        engine: &mut Engine,
        passphrase: &str,
        remember: Option<(&Config, bool)>,
    ) -> io::Result<Option<String>> {
        let key = vault.unlock(account, passphrase).map_err(error)?;
        engine.bind(account, vault)?;
        self.key = Some(key);
        Ok(remember.and_then(|(config, remember)| {
            self.remember_password(config, account, vault, remember.then_some(passphrase))
                .err()
                .map(|e| e.to_string())
        }))
    }
    fn confirm_recovery(
        &mut self,
        config: &Config,
        account: &papertrail_crypto::Account,
        engine: &mut Engine,
        publish: impl FnOnce(&Vault, u64) -> io::Result<()>,
    ) -> io::Result<Option<String>> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| error("No recovery confirmation pending"))?;
        publish(&pending.vault, pending.expected)?;
        self.known_vault = Some((account.clone(), true));
        engine.bind(account, &pending.vault)?;
        self.key = Some(Secret(pending.key.0));
        let warning = self
            .remember_password(
                config,
                account,
                &pending.vault,
                pending.passphrase.as_deref().map(String::as_str),
            )
            .err()
            .map(|e| e.to_string());
        self.pending = None;
        Ok(warning)
    }
    fn clear_identity(&mut self) {
        self.tokens = None;
        self.key = None;
        self.pending = None;
        self.persist_tokens = false;
        self.known_vault = None;
    }
    fn config(&mut self) -> io::Result<Config> {
        let config = crate::read_config(
            &std::fs::read(self.root.join("config.json"))
                .map_err(|_| error("Configure Sync first"))?,
        )?;
        if self.config.as_ref().is_some_and(|old| {
            old.server != config.server
                || old.issuer != config.issuer
                || old.client_id != config.client_id
        }) {
            // GTK and TUI can share a profile: another frontend may have
            // explicitly accepted new settings since this service last ran.
            self.clear_identity();
        }
        self.config = Some(config.clone());
        Ok(config)
    }
    #[cfg(test)]
    fn execute(&mut self, command: Command) -> io::Result<Outcome> {
        self.execute_cancellable(command, &Cancellation::default())
    }
    fn execute_cancellable(
        &mut self,
        command: Command,
        operation: &Cancellation,
    ) -> io::Result<Outcome> {
        check_cancelled(operation)?;
        let (command, remember) = match command {
            Command::Passphrase {
                action,
                passphrase,
                remember,
            } => (
                match action {
                    PassphraseAction::Create => Command::Create(passphrase),
                    PassphraseAction::Unlock => Command::Unlock(passphrase),
                    PassphraseAction::Change => Command::ChangePassphrase(passphrase),
                },
                remember,
            ),
            command => (command, false),
        };
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
            warning: None,
            recovery: None,
            synced: false,
            unlocked: self.key.is_some(),
            settings_update: None,
            local_changed: false,
        };
        if matches!(command, Command::Lock) {
            self.auto_unlock_blocked = true;
            self.key = None;
            self.pending = None;
            outcome.unlocked = false;
            outcome.message = "Vault locked; local plaintext files remain accessible".into();
            return Ok(outcome);
        }
        if matches!(command, Command::Logout) {
            self.auto_unlock_blocked = true;
            self.key = None;
            self.pending = None;
            self.tokens = None;
            outcome.unlocked = false;
            self.persist_tokens = false;
            if !self.memory_only_tokens {
                let config = self.config().or_else(|e| self.config.clone().ok_or(e))?;
                self.forget_credentials(&config, auth::logout)?;
            }
            outcome.message = "Signed out; local notes and rollback checkpoints retained".into();
            return Ok(outcome);
        }
        if matches!(command, Command::Restore)
            && (self.memory_only_tokens || self.auto_unlock_blocked)
        {
            outcome.message =
                "Automatic unlock disabled for this session; sign in/unlock manually".into();
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
        if matches!(
            &command,
            Command::Login | Command::BrowserLogin { .. } | Command::DeviceLogin { .. }
        ) {
            let browser_events = match &command {
                Command::BrowserLogin { events } => Some(events.clone()),
                _ => None,
            };
            let (tokens, cancel, session_only) = match command {
                Command::Login => (
                    auth::login_cancellable(&config, operation)?,
                    Some(operation.clone()),
                    false,
                ),
                Command::BrowserLogin { events } => (
                    auth::login_notified(&config, operation, &events)?,
                    Some(operation.clone()),
                    false,
                ),
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
            check_cancelled(operation)?;
            if let Some(cancel) = &cancel {
                check_cancelled(cancel)?;
            }
            if let Some(events) = &browser_events {
                let _ = events.send(auth::BrowserEvent::Progress(
                    "Validating signed-in account…",
                ));
            }
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
            check_cancelled(operation)?;
            if let Some(cancel) = &cancel {
                check_cancelled(cancel)?;
            }
            if let Some(events) = &browser_events {
                let _ = events.send(auth::BrowserEvent::Progress("Checking vault setup…"));
            }
            let vault = api.vault();
            check_cancelled(operation)?;
            if let Some(cancel) = &cancel {
                check_cancelled(cancel)?;
            }
            // Closing cancellation before touching secure storage avoids a late
            // cancel overwriting/deleting a previously stored session. The UI
            // reports "already approved" if cancellation is now too late.
            if cancel.as_ref().is_some_and(|cancel| !cancel.commit()) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Sign-in cancelled",
                ));
            }
            // Do not publish another account's vault state before committing
            // its login: cancellation must preserve the previous session.
            self.observe_bound_vault(&engine, &account);
            // An outage here does not turn successful login into failure and
            // never implies that the account has no vault.
            let _ = self.observe_vault(&account, vault);
            let session_only = session_only || self.memory_only_tokens;
            self.memory_only_tokens = session_only;
            if let Some(events) = &browser_events {
                let _ = events.send(auth::BrowserEvent::Progress(
                    "Saving sign-in in the system keyring…",
                ));
            }
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
        // Login helpers check discovery themselves. Check here before using
        // cached tokens, refreshing, or contacting any data route.
        auth::config::check(&config)?;
        check_cancelled(operation)?;
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
        check_cancelled(operation)?;
        let account = api.identity()?;
        check_cancelled(operation)?;
        if account.iss != config.issuer {
            return Err(error("API account issuer mismatch"));
        }
        let mut engine = Engine::open(self.root.join("state.json"))?;
        engine.local = self.local.clone();
        self.observe_bound_vault(&engine, &account);
        let vault = api.vault();
        check_cancelled(operation)?;
        let vault = self.observe_vault(&account, vault)?;
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
            Command::Restore => {
                let vault = vault.ok_or_else(|| error("Signed in; create a vault first"))?;
                if self.restore_vault(&config, &account, &vault, &mut engine)? {
                    outcome.message = "Signed in automatically; vault unlocked".into();
                } else {
                    outcome.message = "Signed in; unlock the vault to sync".into();
                }
            }
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
                    passphrase: (remember && !self.memory_only_tokens).then_some(passphrase),
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
                    passphrase: (remember && !self.memory_only_tokens).then_some(passphrase),
                });
                outcome.recovery = Some(recovery.encode());
                outcome.message = "Save the NEW recovery key. Confirmation replaces both wrapping credentials without re-encrypting notes.".into();
            }
            Command::ConfirmRecovery => {
                outcome.warning =
                    self.confirm_recovery(&config, &account, &mut engine, |vault, expected| {
                        api.put_vault(vault, expected)
                    })?;
                outcome.message = "Recovery key confirmed; vault unlocked".into();
            }
            Command::Unlock(passphrase) => {
                let vault = vault.ok_or_else(|| error("Create a vault first"))?;
                outcome.warning = self.unlock_vault(
                    &account,
                    &vault,
                    &mut engine,
                    &passphrase,
                    Some((&config, remember)),
                )?;
                outcome.message = "Vault unlocked for this session".into();
                if self.memory_only_tokens && remember {
                    outcome
                        .message
                        .push_str("; SSH/headless sessions do not save vault passwords");
                }
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
            Command::DeleteFolder { id, settings } => {
                let vault = vault.ok_or_else(|| error("Create a vault first"))?;
                let key = self.key.as_ref().ok_or_else(|| {
                    error("Unlock the vault before deleting a folder from the server")
                })?;
                engine.bind(&account, &vault)?;
                let remote = CancellableRemote {
                    api: &api,
                    cancel: operation,
                };
                engine.delete_folder(&remote, &config, key, &settings, &id)?;
                outcome.message =
                    "Folder deleted from server; local files kept on all devices".into();
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
                let remote = CancellableRemote {
                    api: &api,
                    cancel: operation,
                };
                let result = engine.run(&remote, &config, key, &settings, &protected);
                // A cancelled run may already have applied complete pages. Merge
                // their metadata and refresh the UI just as for a completed run.
                let cancelled = result
                    .as_ref()
                    .is_err_and(|e| e.kind() == io::ErrorKind::Interrupted);
                outcome.message = match result {
                    Ok(message) => message,
                    Err(_) if cancelled => "Sync cancelled; completed changes retained".into(),
                    Err(e) => return Err(e),
                };
                outcome.local_changed = engine.local_changed;
                // Sync metadata back into the existing path-keyed settings model.
                let file = SettingsFile {
                    path: self.settings_path.clone(),
                };
                if file.path.exists() {
                    let mut updated = (*settings).clone();
                    let metadata_config = self.config()?;
                    engine.folder_settings(&metadata_config, &mut updated);
                    for (id, t) in &engine.state.notes {
                        if !engine.note_enabled(id, &metadata_config) {
                            continue;
                        }
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
mod remembered_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_a_folder_pauses_its_ids_without_waiting_for_the_worker_or_changing_the_journal() {
        let temp = tempfile::tempdir().unwrap();
        let notes = temp.path().join("Notes");
        let other = temp.path().join("Other");
        std::fs::create_dir(&notes).unwrap();
        std::fs::create_dir(&other).unwrap();
        std::fs::write(notes.join("keep"), "retained").unwrap();
        let service = Service::new_session_only(&temp.path().join("settings.json"));
        let first = papertrail_crypto::opaque_id();
        let second = papertrail_crypto::opaque_id();
        service
            .configure(Config {
                server: "https://sync.example".into(),
                issuer: "https://issuer.example".into(),
                client_id: "native".into(),
                folders: vec![notes.clone()],
                folder_mappings: [
                    (first.clone(), notes.clone()),
                    (second.clone(), other.clone()),
                ]
                .into(),
                selected_folders: [first.clone(), second.clone()].into(),
                ..Config::default()
            })
            .unwrap();
        let mut engine = Engine::open(temp.path().join("sync/state.json")).unwrap();
        engine.state.catalog = Some(papertrail_crypto::FolderCatalog {
            version: papertrail_crypto::FOLDER_FORMAT,
            folders: [(&first, "Notes"), (&second, "Other")]
                .into_iter()
                .map(|(id, name)| {
                    (
                        id.clone(),
                        papertrail_crypto::FolderData {
                            name: name.into(),
                            color: "preset-0".into(),
                        },
                    )
                })
                .collect(),
            ..Default::default()
        });
        for (id, path) in [(&first, &notes), (&second, &other)] {
            engine.state.bindings.insert(
                id.clone(),
                crate::folders::Binding {
                    path: path.clone(),
                    managed: true,
                    observed_name: String::new(),
                    observed_label: None,
                    observed_color: None,
                },
            );
        }
        engine.save().unwrap();
        let journal = std::fs::read(&engine.state_path).unwrap();
        let control = service.control.lock().unwrap(); // A network worker holds this lock.
        service.pause_folder(&notes).unwrap(); // The journal lock is also held above.
        let paused = service.saved_config().unwrap().unwrap();
        assert!(paused.folders.is_empty());
        assert!(paused.download_dir.is_some());
        assert!(paused.excluded_folders.contains(&first));
        assert!(!paused.selected_folders.contains(&first));
        assert!(paused.selected_folders.contains(&second));
        assert_eq!(paused.folder_mappings[&first], notes);
        assert_eq!(std::fs::read(&engine.state_path).unwrap(), journal);
        assert_eq!(
            std::fs::read_to_string(notes.join("keep")).unwrap(),
            "retained"
        );
        let mut settings = Settings::new(other);
        settings.add_folder(notes.clone(), false); // Reopening is not opt-in to sync.
        assert!(
            !service
                .folder_choices(&settings)
                .unwrap()
                .iter()
                .find(|c| c.id.as_ref() == Some(&first))
                .unwrap()
                .selected
        );
        drop(control);
        drop(engine);
        let mut resumed = paused;
        resumed.excluded_folders.remove(&first);
        resumed.selected_folders.insert(first.clone());
        service.configure(resumed).unwrap();
        assert!(
            service
                .folder_choices(&settings)
                .unwrap()
                .iter()
                .find(|c| c.id.as_ref() == Some(&first))
                .unwrap()
                .selected
        );
        #[cfg(unix)]
        {
            let alias = temp.path().join("Alias");
            std::os::unix::fs::symlink(&notes, &alias).unwrap();
            service.pause_folder(&alias).unwrap();
            assert!(service
                .saved_config()
                .unwrap()
                .unwrap()
                .excluded_folders
                .contains(&first));
        }
    }

    #[test]
    fn closing_local_only_folders_does_not_create_a_sync_profile() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new_session_only(&temp.path().join("settings.json"));
        service.pause_folder(&temp.path().join("Notes")).unwrap();
        assert!(!temp.path().join("sync").exists());
    }
    #[test]
    fn vault_snapshot_distinguishes_unknown_absent_present_and_account_changes() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new_session_only(&temp.path().join("settings.json"));
        let account = papertrail_crypto::Account {
            iss: "https://issuer.example".into(),
            sub: "alice".into(),
        };
        let (vault, _, _) = Vault::create(&account, "a strong test passphrase").unwrap();
        let mut c = service.control.lock().unwrap();
        assert_eq!(service.vault_exists(), None);
        assert!(c.observe_vault(&account, Err(error("offline"))).is_err());
        service.publish_session_state(&c);
        assert_eq!(service.vault_exists(), None);
        c.observe_vault(&account, Ok(None)).unwrap();
        service.publish_session_state(&c);
        assert_eq!(service.vault_exists(), Some(false));
        c.observe_vault(&account, Ok(Some(vault))).unwrap();
        service.publish_session_state(&c);
        assert_eq!(service.vault_exists(), Some(true));
        assert!(c.observe_vault(&account, Err(error("offline"))).is_err());
        service.publish_session_state(&c);
        assert_eq!(service.vault_exists(), Some(true));
        c.observe_vault(&account, Ok(None)).unwrap();
        service.publish_session_state(&c);
        assert_eq!(
            service.vault_exists(),
            Some(true),
            "a lost server vault must not offer a replacement"
        );
        let other = papertrail_crypto::Account {
            sub: "bob".into(),
            ..account
        };
        assert!(c.observe_vault(&other, Err(error("offline"))).is_err());
        service.publish_session_state(&c);
        assert_eq!(service.vault_exists(), None);
        c.observe_vault(&other, Ok(None)).unwrap();
        service.publish_session_state(&c);
        assert_eq!(service.vault_exists(), Some(false));
        c.clear_identity();
        service.publish_session_state(&c);
        assert_eq!(service.vault_exists(), None);
    }

    #[test]
    fn cancelled_login_never_reads_configuration_or_changes_the_session() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new_session_only(&temp.path().join("settings.json"));
        {
            let mut c = service.control.lock().unwrap();
            c.tokens = Some(Tokens {
                access: "existing-access".into(),
                refresh: Some("existing-refresh".into()),
                expires: u64::MAX,
            });
        }
        let cancel = Cancellation::default();
        assert!(cancel.cancel());
        let error = service
            .submit_cancellable(Command::Login, cancel)
            .recv()
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(
            service.signed_in(),
            "cancelling reauthentication must retain the existing session"
        );
        let c = service.control.lock().unwrap();
        assert_eq!(c.tokens.as_ref().unwrap().access, "existing-access");
        assert!(!service.root.join("config.json").exists());
    }

    #[test]
    fn local_operations_do_not_wait_for_the_network_controller_mutex() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new(&temp.path().join("settings.json"));
        std::fs::create_dir_all(&*service.root).unwrap();
        std::fs::write(service.root.join("state.json"), "{}").unwrap();
        let controller = service.control.lock().unwrap();
        assert!(
            !service.signed_in(),
            "menu state must not lock the busy controller"
        );
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
    fn session_snapshot_tracks_lock_errors_and_signout_across_clones() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new_session_only(&temp.path().join("settings.json"));
        let frontend = service.clone();
        assert!(!frontend.signed_in());
        assert!(!frontend.unlocked());
        {
            let mut control = service.control.lock().unwrap();
            control.tokens = Some(Tokens {
                access: "test-access".into(),
                refresh: None,
                expires: u64::MAX,
            });
            control.key = Some(Secret([7; 32]));
            service.publish_session_state(&control);
        }
        assert!(frontend.unlocked());
        assert!(service.submit(Command::Status).recv().unwrap().is_err());
        assert!(
            frontend.unlocked(),
            "failed status must reflect the actual retained key"
        );
        let outcome = service.submit(Command::Lock).recv().unwrap().unwrap();
        assert!(!outcome.unlocked);
        assert!(!frontend.unlocked());
        assert!(frontend.signed_in(), "locking must not sign out");
        assert!(service.submit(Command::Status).recv().unwrap().is_err());
        assert!(
            frontend.signed_in(),
            "an unrelated error must not lose the session"
        );
        service.submit(Command::Logout).recv().unwrap().unwrap();
        assert!(!frontend.signed_in());
        assert!(!frontend.unlocked());
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

    #[test]
    fn discovery_cannot_rebind_an_existing_journal_or_expose_memory_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new_session_only(&temp.path().join("settings.json"));
        let config = Config {
            server: "https://sync.example".into(),
            issuer: "https://issuer.example".into(),
            client_id: "native".into(),
            scopes: "openid offline_access".into(),
            discover_auth: true,
            folders: vec![temp.path().join("notes")],
            ..Config::default()
        };
        service.configure(config.clone()).unwrap();
        let path = service.root.join("state.json");
        {
            let mut engine = Engine::open(path.clone()).unwrap();
            engine.state.account = Some(papertrail_crypto::Account {
                iss: config.issuer.clone(),
                sub: "alice".into(),
            });
            engine.state.vault_id = papertrail_crypto::opaque_id();
            engine.save().unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let mut changed = config.clone();
        changed.issuer = "https://new-issuer.example".into();
        assert!(service
            .configure(changed)
            .unwrap_err()
            .to_string()
            .contains("separate settings profile"));
        assert!(service.saved_config().unwrap().unwrap() == config);
        assert_eq!(std::fs::read(path).unwrap(), before);
        let mut changed = config;
        changed.client_id = "another-native".into();
        assert!(service.configure(changed).is_err());
    }

    #[test]
    fn switching_an_unbound_identity_drops_old_in_memory_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new_session_only(&temp.path().join("settings.json"));
        let mut config = Config {
            server: "https://sync.example".into(),
            issuer: "https://issuer.example".into(),
            client_id: "native".into(),
            scopes: "openid offline_access".into(),
            discover_auth: true,
            folders: vec![temp.path().join("notes")],
            ..Config::default()
        };
        service.configure(config.clone()).unwrap();
        service.control.lock().unwrap().tokens = Some(Tokens {
            access: "ACCESS_SECRET".into(),
            refresh: Some("REFRESH_SECRET".into()),
            expires: u64::MAX,
        });
        service.submit(Command::Status).recv().unwrap().unwrap();
        assert!(service.signed_in());
        let other_frontend = Service::new_session_only(&temp.path().join("settings.json"));
        {
            let mut c = other_frontend.control.lock().unwrap();
            c.config().unwrap();
            c.tokens = Some(Tokens {
                access: "OTHER_ACCESS_SECRET".into(),
                refresh: None,
                expires: u64::MAX,
            });
        }
        config.issuer = "https://new-issuer.example".into();
        service.configure(config).unwrap();
        assert!(service.control.lock().unwrap().tokens.is_none());
        assert!(!service.signed_in());
        {
            let mut c = other_frontend.control.lock().unwrap();
            assert_eq!(c.config().unwrap().issuer, "https://new-issuer.example");
            assert!(c.tokens.is_none());
        }
        let saved = std::fs::read_to_string(service.root.join("config.json")).unwrap();
        assert!(!saved.contains("ACCESS_SECRET"));
        assert!(!saved.contains("REFRESH_SECRET"));
    }
}
