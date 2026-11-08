// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    config::{Paths, Preferences},
    editor::{Editor, Mode},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use papertrail_core::{note::sort_notes, Note, Session, Settings, Store};
use papertrail_storage::{
    startup::{check_notes_folder, prepare_startup_folder},
    Repository, SettingsFile,
};
use papertrail_sync::control::{Command as SyncCommand, Outcome, Service};
use ratatui::{
    layout::{Position, Rect},
    widgets::ListState,
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    Folders,
    #[default]
    Notes,
    Editor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Command,
    NotesSearch,
    Find,
    Delete,
    Unlock,
    CreateVault,
    RecoverVault,
    ChangePassphrase,
    ConfirmRecovery,
}
impl PromptKind {
    pub fn secret(self) -> bool {
        matches!(
            self,
            Self::Unlock | Self::CreateVault | Self::RecoverVault | Self::ChangePassphrase
        )
    }
}
pub struct Prompt {
    pub kind: PromptKind,
    pub text: String,
    pub completion: crate::completion::Completion,
}

pub struct App {
    pub sync: Service,
    pub sync_job: Option<std::sync::mpsc::Receiver<io::Result<Outcome>>>,
    pub sync_unlocked: bool,
    pub sync_last: Instant,
    pub recovery_key: Option<Zeroizing<String>>,
    pub device_prompt: Option<papertrail_sync::auth::device::Prompt>,
    pub device_details: bool,
    pub(crate) device_notifications:
        Option<std::sync::mpsc::Receiver<papertrail_sync::auth::device::Prompt>>,
    pub(crate) device_cancel: Option<papertrail_sync::auth::device::Cancellation>,
    after_login: Option<SyncCommand>,
    pub settings: Settings,
    baseline: Settings,
    file: SettingsFile,
    home: PathBuf,
    preferences_path: PathBuf,
    pub preferences: Preferences,
    pub session: Session,
    pub editor: Editor,
    pub notes: Vec<Note>,
    pub note_index: usize,
    pub folder_index: usize,
    pub all: bool,
    pub focus: Focus,
    pub prompt: Option<Prompt>,
    pub query: String,
    pub find: String,
    pub status: String,
    pub help: bool,
    pub quit: bool,
    pub external: bool,
    pub autosave_paused: bool,
    pub help_scroll: u16,
    pub mouse_enabled: bool,
    pub folders_visible: bool,
    pub folder_state: ListState,
    pub note_state: ListState,
    pub panes: [Rect; 3],
    pub path_details: Option<String>,
}

impl App {
    pub fn new(paths: Paths) -> io::Result<Self> {
        let file = SettingsFile {
            path: paths.settings,
        };
        let mut settings = file.load(paths.default_notes.clone(), &paths.home)?;
        let baseline = settings.clone();
        if let Some(folder) = paths.folder {
            check_notes_folder(&folder)?;
            settings.add_folder(fs::canonicalize(folder)?, true);
        }
        let recovery = prepare_startup_folder(&mut settings, &paths.default_notes)?;
        let preferences_path = file.path.with_file_name("tui.json");
        let preferences = Preferences::load(&preferences_path)?;
        let mut app = Self {
            sync: if session_only_login() {
                Service::new_session_only(&file.path)
            } else {
                Service::new(&file.path)
            },
            sync_job: None,
            sync_unlocked: false,
            sync_last: Instant::now(),
            recovery_key: None,
            device_prompt: None,
            device_details: false,
            device_notifications: None,
            device_cancel: None,
            after_login: None,
            folder_index: settings
                .note_folders
                .iter()
                .position(|p| *p == settings.notes_dir)
                .unwrap_or(0),
            settings,
            home: paths.home,
            baseline,
            file,
            preferences_path,
            preferences,
            session: Session::default(),
            editor: Editor::default(),
            notes: vec![],
            note_index: 0,
            all: false,
            focus: Focus::Notes,
            prompt: None,
            query: String::new(),
            find: String::new(),
            status: "? help | Tab panes | : commands".into(),
            help: false,
            quit: false,
            external: false,
            autosave_paused: false,
            help_scroll: 0,
            mouse_enabled: !paths.no_mouse,
            folders_visible: true,
            folder_state: ListState::default(),
            note_state: ListState::default(),
            panes: [Rect::default(); 3],
            path_details: None,
        };
        app.refresh()?;
        if let Some(recovery) = recovery {
            app.status = format!(
                "Folder unavailable: {}. Using {}. :folder PATH to restore access.",
                recovery.unavailable.display(),
                recovery.fallback.display()
            );
        }
        Ok(app)
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        if self.focus == Focus::Notes {
            self.flush()?;
        }
        let previous = self.notes.get(self.note_index).map(|n| n.path.clone());
        let folders = if self.all || !self.query.is_empty() {
            self.settings.note_folders.clone()
        } else {
            vec![self.settings.notes_dir.clone()]
        };
        let mut notes = vec![];
        let mut unavailable = vec![];
        for folder in folders {
            match Repository.list(&folder) {
                Ok(listed) => {
                    for note in listed {
                        if self.query.is_empty()
                            || Repository.load(&note.path)?.matches(&self.query)
                        {
                            notes.push(note);
                        }
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound
                            | io::ErrorKind::PermissionDenied
                            | io::ErrorKind::NotADirectory
                    ) =>
                {
                    unavailable.push(folder)
                }
                Err(e) => return Err(e),
            }
        }
        sort_notes(&mut notes, &self.settings.pinned_notes);
        notes.dedup_by(|a, b| a.path == b.path);
        self.notes = notes;
        self.note_index = previous
            .and_then(|p| self.notes.iter().position(|n| n.path == p))
            .unwrap_or(self.note_index)
            .min(self.notes.len().saturating_sub(1));
        if !unavailable.is_empty() {
            self.status = format!(
                "{} unavailable folder(s); :folder PATH to restore access",
                unavailable.len()
            );
        }
        if self.focus == Focus::Notes {
            self.preview_selected()?;
        } else if let Some(path) = self.session.current.as_ref().map(|n| &n.path) {
            if let Some(index) = self.notes.iter().position(|n| &n.path == path) {
                self.note_index = index;
            }
        }
        Ok(())
    }

    pub fn persist(&mut self) -> io::Result<()> {
        let merged = self
            .file
            .save_merged(&self.settings, &self.baseline, &self.home)?;
        self.settings = merged.clone();
        self.baseline = merged;
        self.preferences.save(&self.preferences_path)
    }
    pub fn flush(&mut self) -> io::Result<()> {
        self.session.flush(&Repository)?;
        self.autosave_paused = false;
        Ok(())
    }
    pub fn tick(&mut self) -> io::Result<()> {
        self.sync_poll()?;
        if self.sync_unlocked
            && self.sync_job.is_none()
            && self.sync_last.elapsed() > std::time::Duration::from_secs(30)
        {
            self.sync_now()?;
        }
        if self.sync_blocks_input() {
            return Ok(());
        }
        if !self.autosave_paused && self.session.save_due(Instant::now()) {
            if let Err(error) = self.flush() {
                self.autosave_paused = true;
                return Err(error);
            }
            self.refresh()?;
            self.status = "Saved".into();
        }
        Ok(())
    }
    fn sync_edit(&mut self) {
        let text = self.editor.text();
        if text != self.session.text {
            self.session.edit(text, Instant::now());
        }
    }
    pub fn paste(&mut self, text: &str) {
        if self.sync_blocks_input() || self.device_details {
            return;
        }
        if let Some(prompt) = &mut self.prompt {
            prompt.completion.reset();
            // Pasting a command never executes it; require an explicit Enter.
            prompt.text.extend(text.chars().filter(|c| !c.is_control()));
        } else if self.focus == Focus::Editor
            && self.editor.mode == Mode::Insert
            && self.session.current.is_some()
        {
            self.editor.paste_text(text);
            self.sync_edit();
        }
    }
    pub fn open(&mut self, path: &Path) -> io::Result<()> {
        self.preview(path)?;
        if let Some(folder) = path.parent() {
            self.settings.add_folder(folder.to_owned(), true);
            self.folder_index = self
                .settings
                .note_folders
                .iter()
                .position(|p| p == folder)
                .unwrap_or(0);
        }
        self.focus = Focus::Editor;
        Ok(())
    }

    fn preview(&mut self, path: &Path) -> io::Result<()> {
        if self
            .session
            .current
            .as_ref()
            .is_some_and(|n| n.path == path)
        {
            return Ok(());
        }
        self.session.open(&Repository, path)?;
        self.editor.set_text(&self.session.text);
        self.autosave_paused = false;
        Ok(())
    }

    fn preview_selected(&mut self) -> io::Result<()> {
        if let Some(path) = self.notes.get(self.note_index).map(|n| n.path.clone()) {
            self.preview(&path)?;
        } else {
            self.session.close(&Repository)?;
            self.editor.set_text("");
        }
        Ok(())
    }

    pub fn select_note(&mut self, index: usize) -> io::Result<()> {
        let Some(path) = self.notes.get(index).map(|n| n.path.clone()) else {
            return Ok(());
        };
        self.preview(&path)?;
        self.note_index = index;
        Ok(())
    }

    fn select_folder(&mut self, index: usize) -> io::Result<()> {
        let Some(folder) = self.settings.note_folders.get(index).cloned() else {
            return Ok(());
        };
        check_notes_folder(&folder)?;
        self.flush()?;
        self.settings.notes_dir = folder;
        self.folder_index = index;
        self.all = false;
        self.query.clear();
        self.focus = Focus::Notes;
        self.editor.mode = Mode::Normal;
        self.editor.pending = None;
        self.editor.area.cancel_selection();
        self.refresh()
    }

    fn switch_pane(&mut self, backwards: bool) -> io::Result<()> {
        let next = match (self.focus, backwards) {
            (Focus::Folders, false) | (Focus::Editor, true) => Focus::Notes,
            (Focus::Notes, false) | (Focus::Folders, true) => Focus::Editor,
            _ if self.folders_visible => Focus::Folders,
            (Focus::Editor, false) => Focus::Notes,
            _ => Focus::Editor,
        };
        if next == Focus::Notes {
            self.preview_selected()?;
        }
        self.editor.mode = Mode::Normal;
        self.editor.pending = None;
        self.editor.area.cancel_selection();
        self.focus = next;
        Ok(())
    }

    pub fn mouse(&mut self, event: MouseEvent) -> io::Result<()> {
        if self.sync_blocks_input() || self.recovery_key.is_some() || self.device_details {
            return Ok(());
        }
        if !self.mouse_enabled || self.prompt.is_some() {
            return Ok(());
        }
        let step = match event.kind {
            MouseEventKind::ScrollDown => 3,
            MouseEventKind::ScrollUp => -3,
            _ => 0,
        };
        if self.help {
            self.help_scroll = self.help_scroll.saturating_add_signed(step).min(200);
            return Ok(());
        }
        let point = Position::new(event.column, event.row);
        let Some(pane) = self.panes.iter().position(|rect| rect.contains(point)) else {
            return Ok(());
        };
        let rect = self.panes[pane];
        if step != 0 {
            match pane {
                0 => {
                    self.folder_index = self
                        .folder_index
                        .saturating_add_signed(step as isize)
                        .min(self.settings.note_folders.len().saturating_sub(1))
                }
                1 => {
                    self.select_note(
                        self.note_index
                            .saturating_add_signed(step as isize)
                            .min(self.notes.len().saturating_sub(1)),
                    )?;
                    self.focus = Focus::Notes;
                    self.editor.mode = Mode::Normal;
                    self.editor.area.cancel_selection();
                }
                _ => self.editor.scroll(step),
            }
        } else if event.kind == MouseEventKind::Down(MouseButton::Left) {
            let focus = [Focus::Folders, Focus::Notes, Focus::Editor][pane];
            let inner = rect.inner(ratatui::layout::Margin::new(1, 1));
            if inner.contains(point) {
                let row = usize::from(event.row - inner.y);
                match pane {
                    0 => {
                        self.select_folder(self.folder_state.offset() + row)?;
                        self.focus = Focus::Folders;
                        return Ok(());
                    }
                    1 => self.select_note(self.note_state.offset() + row / 2)?,
                    _ => self
                        .editor
                        .click(event.row - inner.y, event.column - inner.x),
                }
            }
            if focus == Focus::Notes {
                self.preview_selected()?;
            }
            if focus != self.focus {
                self.editor.mode = Mode::Normal;
                self.editor.pending = None;
                self.editor.area.cancel_selection();
            }
            self.focus = focus;
        }
        Ok(())
    }
    pub fn current_path(&self) -> io::Result<PathBuf> {
        self.session
            .current
            .as_ref()
            .map(|n| n.path.clone())
            .ok_or_else(|| io::Error::other("Open a note first"))
    }
    pub fn reload(&mut self, discard: bool) -> io::Result<()> {
        if self.session.is_dirty() && !discard {
            return Err(io::Error::other(
                "Unsaved changes. Use :saveas PATH, or :reload! to discard explicitly.",
            ));
        }
        let note = Repository.load(&self.current_path()?)?;
        self.session = Session::default();
        self.autosave_paused = false;
        self.session.open(&Repository, &note.path)?;
        self.editor.set_text(&self.session.text);
        Ok(())
    }
    fn new_note(&mut self, name: &str) -> io::Result<()> {
        self.flush()?;
        check_notes_folder(&self.settings.notes_dir)?;
        let mut note = Repository.create(&self.settings.notes_dir)?;
        if !name.is_empty() {
            note = Repository.rename(&note.path, name)?;
        }
        self.open(&note.path)?;
        self.refresh()?;
        self.editor.mode = Mode::Insert;
        Ok(())
    }
    fn start_prompt(&mut self, kind: PromptKind) {
        self.prompt = Some(Prompt {
            kind,
            text: String::new(),
            completion: crate::completion::Completion::default(),
        });
    }
    fn finish(&mut self, force: bool) -> io::Result<()> {
        if !force {
            self.flush()?;
            self.persist()?;
        }
        if let Some(cancel) = &self.device_cancel {
            cancel.cancel();
        }
        self.quit = true;
        Ok(())
    }
    pub fn key(&mut self, key: KeyEvent) -> io::Result<()> {
        if self.device_cancel.is_some()
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('c')
        {
            self.cancel_device_login();
            return Ok(());
        }
        if self.device_details {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return self.finish(false);
            }
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.device_details = false;
                    self.help_scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = (self.help_scroll + 1).min(200)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => self.help_scroll = (self.help_scroll + 10).min(200),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                _ => (),
            }
            return Ok(());
        }
        if self.sync_blocks_input() {
            self.status = "Sync operation running; editing temporarily paused".into();
            return Ok(());
        }
        if self.help {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = (self.help_scroll + 1).min(200)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => self.help_scroll = (self.help_scroll + 10).min(200),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                KeyCode::Home => self.help_scroll = 0,
                _ => {
                    self.help = false;
                    self.help_scroll = 0;
                    self.path_details = None;
                }
            }
            return Ok(());
        }
        if self.prompt.is_some() {
            return self.prompt_key(key);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return self.finish(false),
                KeyCode::Char('s') => {
                    self.flush()?;
                    self.persist()?;
                    self.status = "Saved".into();
                    return Ok(());
                }
                KeyCode::Char('n') => return self.new_note(""),
                KeyCode::Char('c') => {
                    self.editor.mode = Mode::Normal;
                    self.editor.area.cancel_selection();
                    self.status = "Use :q to save and quit, or :q! to discard".into();
                    return Ok(());
                }
                _ => (),
            }
        }
        let typing = self.focus == Focus::Editor && self.editor.mode == Mode::Insert;
        if !typing {
            match key.code {
                KeyCode::Char('H' | 'L') => {
                    return self.switch_pane(key.code == KeyCode::Char('H'))
                }
                KeyCode::Char(':') => {
                    self.start_prompt(PromptKind::Command);
                    return Ok(());
                }
                KeyCode::Char('/') => {
                    self.start_prompt(if self.focus == Focus::Editor {
                        PromptKind::Find
                    } else {
                        PromptKind::NotesSearch
                    });
                    return Ok(());
                }
                KeyCode::Char('?') => {
                    self.help = true;
                    return Ok(());
                }
                _ => (),
            }
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.switch_pane(key.code == KeyCode::BackTab)?;
            }
            _ if self.focus == Focus::Editor => {
                if self.session.current.is_none() {
                    self.status = ":new to create a note".into();
                    return Ok(());
                }
                if !typing && matches!(key.code, KeyCode::Char('n' | 'N')) {
                    let forward = key.code == KeyCode::Char('n');
                    if !self.find.is_empty() && !self.editor.find(&self.find, forward) {
                        self.status = "No match".into();
                    }
                } else {
                    self.editor.key(key);
                    self.sync_edit();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.focus == Focus::Folders {
                    self.folder_index = (self.folder_index + 1)
                        .min(self.settings.note_folders.len().saturating_sub(1));
                } else {
                    self.select_note(
                        (self.note_index + 1).min(self.notes.len().saturating_sub(1)),
                    )?;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.focus == Focus::Folders {
                    self.folder_index = self.folder_index.saturating_sub(1);
                } else {
                    self.select_note(self.note_index.saturating_sub(1))?;
                }
            }
            KeyCode::Enter => {
                if self.focus == Focus::Folders {
                    self.select_folder(self.folder_index)?;
                } else if let Some(path) = self.notes.get(self.note_index).map(|n| n.path.clone()) {
                    self.open(&path)?;
                }
            }
            KeyCode::Char('p') => {
                if let Some(path) = self.notes.get(self.note_index).map(|n| n.path.clone()) {
                    self.pin(&path)?;
                }
            }
            KeyCode::Esc => {
                self.query.clear();
                self.refresh()?;
            }
            _ => (),
        }
        Ok(())
    }
    fn prompt_key(&mut self, key: KeyEvent) -> io::Result<()> {
        if key.code == KeyCode::Esc {
            if let Some(prompt) = &mut self.prompt {
                if prompt.kind.secret() {
                    prompt.text.zeroize();
                }
            }
            if self.recovery_key.take().is_some() {
                self.sync_start(SyncCommand::Lock)?;
            }
            self.prompt = None;
            return Ok(());
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            let prompt = self.prompt.as_mut().unwrap();
            if prompt.kind == PromptKind::Command {
                prompt.completion.cycle(
                    &mut prompt.text,
                    key.code == KeyCode::BackTab,
                    self.settings.note_folders.len(),
                    &std::env::current_dir()?,
                    &self.home,
                )?;
            }
            return Ok(());
        }
        if key.code == KeyCode::Enter {
            let prompt = self.prompt.take().unwrap();
            match prompt.kind {
                PromptKind::Command => self.command(&prompt.text)?,
                PromptKind::NotesSearch => {
                    self.query = prompt.text;
                    self.focus = Focus::Notes;
                    self.refresh()?;
                }
                PromptKind::Find => {
                    self.find = prompt.text;
                    if !self.editor.find(&self.find, true) {
                        self.status = "No match".into();
                    }
                }
                PromptKind::Delete => {
                    if prompt.text == "yes" {
                        self.flush()?;
                        let path = self.current_path()?;
                        Repository.delete(&path)?;
                        self.session.discard_deleted(&path);
                        self.settings.delete_note(&path);
                        self.editor.set_text("");
                        self.focus = Focus::Notes;
                        self.persist()?;
                        self.refresh()?;
                    }
                }
                PromptKind::Unlock
                | PromptKind::CreateVault
                | PromptKind::RecoverVault
                | PromptKind::ChangePassphrase => {
                    let secret = Zeroizing::new(prompt.text);
                    let command = match prompt.kind {
                        PromptKind::Unlock => SyncCommand::Unlock(secret),
                        PromptKind::CreateVault => SyncCommand::Create(secret),
                        PromptKind::RecoverVault => SyncCommand::Recover(secret),
                        _ => SyncCommand::ChangePassphrase(secret),
                    };
                    self.sync_start(command)?;
                }
                PromptKind::ConfirmRecovery => {
                    self.recovery_key = None;
                    self.sync_start(if prompt.text == "yes" {
                        SyncCommand::ConfirmRecovery
                    } else {
                        SyncCommand::Lock
                    })?;
                }
            }
        } else if let Some(prompt) = &mut self.prompt {
            match key.code {
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    prompt.completion.reset();
                    prompt.text.push(c)
                }
                KeyCode::Backspace => {
                    prompt.completion.reset();
                    prompt.text.pop();
                }
                _ => (),
            }
        }
        Ok(())
    }
    fn pin(&mut self, path: &Path) -> io::Result<()> {
        self.settings
            .set_pinned(path, !self.settings.pinned_notes.contains(&path.to_owned()));
        self.persist()?;
        self.refresh()
    }
    pub fn command(&mut self, input: &str) -> io::Result<()> {
        let (command, arg) = input.trim().split_once(' ').unwrap_or((input.trim(), ""));
        let arg = arg.trim();
        match command {
            "sync-configure" => {
                let parts = shell_words::split(arg).map_err(io::Error::other)?;
                if parts.len() < 3 || parts.len() > 4 {
                    return Err(io::Error::other("Usage: :sync-configure HTTPS_SERVER HTTPS_ISSUER PUBLIC_CLIENT_ID 'openid offline_access API_SCOPE'; the quoted scopes argument is optional; do not type square brackets"));
                }
                let saved = self.sync.saved_config()?;
                let fresh = saved.is_none();
                let mut config = saved.unwrap_or_default();
                config.version = 2;
                config.server = parts[0].clone();
                config.issuer = parts[1].clone();
                config.client_id = parts[2].clone();
                config.scopes = parts.get(3).cloned().unwrap_or_else(|| {
                    if config.scopes.is_empty() {
                        "openid offline_access".into()
                    } else {
                        config.scopes.clone()
                    }
                });
                if fresh {
                    config.folders = vec![self.settings.notes_dir.clone()];
                }
                self.sync.configure(config)?;
                self.status =
                    "Sync configured for active folder. :login, then :create-vault or :unlock"
                        .into();
            }
            "sync-folders" => {
                let choices = self.sync.folder_choices(&self.settings)?;
                self.path_details = Some(
                    choices
                        .iter()
                        .map(|f| {
                            format!(
                                "[{}] {}\n  ID: {}\n  Local: {}",
                                if f.selected { "x" } else { " " },
                                f.name,
                                f.id.as_deref().unwrap_or("not registered yet"),
                                f.path
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| "automatic download directory".into())
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                );
                self.help = true;
                self.help_scroll = 0;
            }
            "sync-auto-download" => {
                let mut config = self
                    .sync
                    .saved_config()?
                    .ok_or_else(|| io::Error::other("Configure Sync first"))?;
                config.auto_download = match arg {
                    "on" => true,
                    "off" => false,
                    _ => return Err(io::Error::other("Use :sync-auto-download on or off")),
                };
                self.sync.configure(config)?;
                self.status = "Remote-folder download policy saved".into();
            }
            "sync-select-folder" | "sync-unselect-folder" => {
                let choices = self.sync.folder_choices(&self.settings)?;
                let choice = choices
                    .iter()
                    .find(|f| f.id.as_deref() == Some(arg))
                    .ok_or_else(|| {
                        io::Error::other("Unknown remote folder ID; use :sync-folders")
                    })?;
                let mut config = self
                    .sync
                    .saved_config()?
                    .ok_or_else(|| io::Error::other("Configure Sync first"))?;
                if command == "sync-select-folder" {
                    config.selected_folders.insert(arg.into());
                    config.excluded_folders.remove(arg);
                } else {
                    config.selected_folders.remove(arg);
                    config.excluded_folders.insert(arg.into());
                    if let Some(path) = &choice.path {
                        config
                            .folders
                            .retain(|p| !papertrail_sync::same_directory(p, path));
                    }
                }
                self.sync.configure(config)?;
                self.status = "Folder selection saved; no notes deleted".into();
            }
            "sync-add-folder" | "sync-remove-folder" | "sync-download-dir" => {
                let mut config = self
                    .sync
                    .saved_config()?
                    .ok_or_else(|| io::Error::other("Configure Sync first"))?;
                let path = self.folder_arg(arg)?;
                let path = if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir()?.join(path)
                };
                if command == "sync-download-dir" {
                    config.download_dir = Some(path);
                } else {
                    let path = fs::canonicalize(path)?;
                    check_notes_folder(&path)?;
                    if command == "sync-add-folder" {
                        if !config
                            .folders
                            .iter()
                            .any(|p| papertrail_sync::same_directory(p, &path))
                        {
                            config.folders.push(path.clone());
                        }
                    } else {
                        config.download_dir = Some(config.download_root()?);
                        config
                            .folders
                            .retain(|p| !papertrail_sync::same_directory(p, &path));
                    }
                    for choice in self.sync.folder_choices(&self.settings)? {
                        if choice
                            .path
                            .as_ref()
                            .is_some_and(|p| papertrail_sync::same_directory(p, &path))
                        {
                            if let Some(id) = choice.id {
                                if command == "sync-add-folder" {
                                    config.excluded_folders.remove(&id);
                                } else {
                                    config.excluded_folders.insert(id);
                                }
                            }
                        }
                    }
                }
                self.sync.configure(config)?;
                self.status =
                    "Sync folder selection saved; local and remote notes were kept".into();
            }
            "sync-map-folder" | "sync-map-legacy" => {
                let parts = shell_words::split(arg).map_err(io::Error::other)?;
                if parts.len() != 2 {
                    return Err(io::Error::other("Usage: :sync-map-folder FOLDER_ID '/absolute/path' or :sync-map-legacy NOTE_ID FOLDER_ID"));
                }
                let mut config = self
                    .sync
                    .saved_config()?
                    .ok_or_else(|| io::Error::other("Configure Sync first"))?;
                if command == "sync-map-legacy" {
                    config
                        .legacy_note_mappings
                        .insert(parts[0].clone(), parts[1].clone());
                } else {
                    let path = fs::canonicalize(&parts[1])?;
                    check_notes_folder(&path)?;
                    if self.focus == Focus::Editor {
                        return Err(io::Error::other(
                            "Leave the editor before changing folder mappings",
                        ));
                    }
                    self.flush()?;
                    self.session = Session::default();
                    config
                        .folder_mappings
                        .insert(parts[0].clone(), path.clone());
                    config.excluded_folders.remove(&parts[0]);
                    if !config.folders.contains(&path) {
                        config.folders.push(path);
                    }
                }
                self.sync.configure(config)?;
                self.status = "Folder assignment confirmed. :sync to apply it without removing original files.".into();
            }
            "sync-accept-remote" => {
                self.sync.accept_remote_folder(arg, &self.settings)?;
                self.status = "Remote folder metadata accepted; local directory kept".into();
            }
            "sync" => self.sync_now()?,
            "sync-status" => self.sync_start(SyncCommand::Status)?,
            "login" => {
                if !arg.is_empty() {
                    return Err(io::Error::other(
                        "Use :login without arguments; approve the displayed code in your browser",
                    ));
                }
                self.login_device()?;
            }
            "login-browser" => {
                if !arg.is_empty() {
                    return Err(io::Error::other("Use :login-browser without arguments"));
                }
                self.sync_start(SyncCommand::Login)?;
            }
            "login-cancel" => self.cancel_device_login(),
            "login-show" => {
                if self.device_prompt.is_some() {
                    self.device_details = true;
                    self.help_scroll = 0;
                } else {
                    self.status = "No device code to display; use :login".into();
                }
            }
            "logout" => {
                self.sync_unlocked = false;
                self.recovery_key = None;
                if self.device_cancel.is_some() {
                    self.cancel_device_login();
                    self.after_login = Some(SyncCommand::Logout);
                } else {
                    self.sync_start(SyncCommand::Logout)?;
                }
            }
            "lock" => {
                self.sync_unlocked = false;
                self.recovery_key = None;
                if self.device_cancel.is_some() {
                    self.cancel_device_login();
                    self.after_login = Some(SyncCommand::Lock);
                } else {
                    self.sync_start(SyncCommand::Lock)?;
                }
            }
            "unlock" | "create-vault" | "recover-vault" | "change-passphrase" => {
                if !arg.is_empty() {
                    return Err(io::Error::other("Never enter a passphrase or recovery key as a command argument; use the masked prompt"));
                }
                self.start_prompt(match command {
                    "unlock" => PromptKind::Unlock,
                    "create-vault" => PromptKind::CreateVault,
                    "recover-vault" => PromptKind::RecoverVault,
                    _ => PromptKind::ChangePassphrase,
                });
            }
            "q" | "quit" => return self.finish(false),
            "q!" => return self.finish(true),
            "w" => {
                self.flush()?;
                self.persist()?;
                self.status = "Saved".into();
            }
            "wq" | "x" => return self.finish(false),
            "new" => self.new_note(arg)?,
            "rename" => {
                if arg.is_empty() {
                    return Err(io::Error::other("Usage: :rename NAME"));
                }
                self.flush()?;
                let old = self.current_path()?;
                let note = Repository.rename(&old, arg)?;
                self.sync.relocate(&old, &note.path)?;
                self.settings.rename_note(&old, &note.path);
                self.session.current = Some(note);
                self.persist()?;
                self.refresh()?;
            }
            "move" => {
                self.flush()?;
                let folder = self.folder_arg(arg)?;
                check_notes_folder(&folder)?;
                let old = self.current_path()?;
                let note = Repository.move_note(&old, &folder)?;
                self.sync.relocate(&old, &note.path)?;
                self.settings.rename_note(&old, &note.path);
                self.session.current = Some(note.clone());
                self.open(&note.path)?;
                self.persist()?;
                self.refresh()?;
            }
            "folder" => {
                self.flush()?;
                let folder = self.folder_arg(arg)?;
                check_notes_folder(&folder)?;
                self.settings.add_folder(fs::canonicalize(folder)?, true);
                self.folder_index = self
                    .settings
                    .note_folders
                    .iter()
                    .position(|p| *p == self.settings.notes_dir)
                    .unwrap_or(0);
                self.all = false;
                self.query.clear();
                self.focus = Focus::Notes;
                self.refresh()?;
                self.persist()?;
            }
            "close-folder" => {
                let folder = self.settings.notes_dir.clone();
                let next = self
                    .settings
                    .note_folders
                    .iter()
                    .find(|p| **p != folder && check_notes_folder(p).is_ok())
                    .cloned()
                    .ok_or_else(|| io::Error::other("Cannot close the last available folder"))?;
                self.flush()?;
                self.settings.remove_folder(&folder);
                self.settings.notes_dir = next;
                self.folder_index = 0;
                self.focus = Focus::Notes;
                self.refresh()?;
                self.persist()?;
            }
            "folder-up" | "folder-down" => {
                let index = self.folder_index;
                let next = if command == "folder-up" {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(self.settings.note_folders.len() - 1)
                };
                self.settings.note_folders.swap(index, next);
                self.folder_index = next;
                self.persist()?;
            }
            "all" => {
                self.all = true;
                self.focus = Focus::Notes;
                self.refresh()?;
            }
            "pin" => {
                let path = self.current_path()?;
                self.pin(&path)?;
            }
            "color" => {
                if !valid_color(arg) {
                    return Err(io::Error::other(
                        "Use :color preset-0 through preset-11, or #rrggbb",
                    ));
                }
                self.settings
                    .folder_colors
                    .insert(self.settings.notes_dir.clone(), arg.to_ascii_lowercase());
                self.persist()?;
            }
            "delete" => {
                self.current_path()?;
                self.start_prompt(PromptKind::Delete);
            }
            "edit" => {
                self.flush()?;
                self.current_path()?;
                self.external = true;
            }
            "reload" | "reload!" => {
                self.reload(command.ends_with('!'))?;
                self.refresh()?;
            }
            "refresh" => self.refresh()?,
            "search" => {
                self.query = arg.to_owned();
                self.focus = Focus::Notes;
                self.refresh()?;
            }
            "saveas" => self.save_as(arg)?,
            "replace" => {
                let parts = shell_words::split(arg).map_err(io::Error::other)?;
                if parts.len() != 2 || parts[0].is_empty() {
                    return Err(io::Error::other(
                        "Usage: :replace 'find text' 'replacement' (all literal matches)",
                    ));
                }
                self.current_path()?;
                let text = self.editor.text().replace(&parts[0], &parts[1]);
                self.editor.replace_text(&text);
                self.sync_edit();
            }
            "set" => {
                match arg {
                    "number" => self.preferences.hide_numbers = false,
                    "nonumber" => self.preferences.hide_numbers = true,
                    _ if arg.starts_with("folder-width ") => {
                        self.preferences.folder_width = arg[13..]
                            .trim()
                            .parse::<u16>()
                            .ok()
                            .filter(|n| (10..=30).contains(n))
                            .ok_or_else(|| {
                                io::Error::other("Use :set folder-width 10 through 30")
                            })?;
                    }
                    _ => {
                        return Err(io::Error::other(
                            "Use :set number, nonumber, or folder-width N (10–30)",
                        ))
                    }
                }
                self.preferences.save(&self.preferences_path)?;
            }
            "path" => {
                self.status = if self.focus == Focus::Folders {
                    self.settings
                        .note_folders
                        .get(self.folder_index)
                        .unwrap_or(&self.settings.notes_dir)
                        .display()
                        .to_string()
                } else {
                    self.current_path()
                        .unwrap_or_else(|_| self.settings.notes_dir.clone())
                        .display()
                        .to_string()
                };
                self.path_details = Some(self.status.clone());
                self.help_scroll = 0;
                self.help = true;
            }
            "help" => self.help = true,
            "" => (),
            _ => {
                return Err(io::Error::other(format!(
                    "Unknown command: {command}. :help for commands"
                )))
            }
        }
        Ok(())
    }

    fn sync_start(&mut self, command: SyncCommand) -> io::Result<()> {
        if self.sync_job.is_some() {
            return Err(io::Error::other("Sync operation already running"));
        }
        // Lock/sign-out must remain available even if a local note cannot save.
        // They do not discard the editor buffer or change plaintext note files.
        if !matches!(&command, SyncCommand::Lock | SyncCommand::Logout) {
            self.flush()?;
            self.persist()?;
        }
        let message = match &command {
            SyncCommand::DeviceLogin { .. } => "Requesting a device login code…",
            SyncCommand::Login => "Opening the system browser for sign-in…",
            _ => "Sync operation running…",
        };
        self.sync_job = Some(self.sync.submit(command));
        self.sync_last = Instant::now();
        self.status = message.into();
        Ok(())
    }
    fn sync_blocks_input(&self) -> bool {
        self.sync_job.is_some() && self.device_cancel.is_none()
    }
    fn login_device(&mut self) -> io::Result<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = papertrail_sync::auth::device::Cancellation::default();
        // SSH/headless sessions must not wait for a desktop keyring unlock prompt.
        self.sync_start(SyncCommand::DeviceLogin {
            notifications: tx,
            cancel: cancel.clone(),
            session_only: session_only_login(),
        })?;
        self.device_cancel = Some(cancel);
        self.device_notifications = Some(rx);
        self.device_prompt = None;
        self.device_details = false;
        Ok(())
    }
    fn cancel_device_login(&mut self) {
        if let Some(cancel) = &self.device_cancel {
            self.status = if cancel.cancel() {
                "Cancelling device sign-in… Editing remains available."
            } else {
                "Sign-in is already approved and finishing. Use :logout to clear the session."
            }
            .into();
        } else {
            self.status = "No device sign-in is running".into();
        }
        self.device_prompt = None;
        self.device_details = false;
    }
    fn sync_now(&mut self) -> io::Result<()> {
        self.sync_start(SyncCommand::Sync {
            settings: Box::new(self.settings.clone()),
            protected: self
                .session
                .current
                .as_ref()
                .filter(|_| self.focus == Focus::Editor)
                .map(|n| vec![n.path.clone()])
                .unwrap_or_default(),
        })
    }
    fn sync_poll(&mut self) -> io::Result<()> {
        if let Some(rx) = &self.device_notifications {
            while let Ok(prompt) = rx.try_recv() {
                if self
                    .device_cancel
                    .as_ref()
                    .is_some_and(|cancel| !cancel.is_cancelled())
                    && prompt.expires_at > Instant::now()
                {
                    self.device_prompt = Some(prompt);
                }
            }
        }
        let result = self.sync_job.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err(io::Error::other("Sync worker disconnected"))),
        });
        let Some(result) = result else {
            return Ok(());
        };
        self.sync_job = None;
        let device = self.device_cancel.take();
        self.device_notifications = None;
        self.device_prompt = None;
        self.device_details = false;
        self.sync_last = Instant::now();
        let after_login = self.after_login.take();
        match result {
            Ok(result) => {
                let was_unlocked = self.sync_unlocked;
                self.sync_unlocked = result.unlocked && after_login.is_none();
                self.status = result.message;
                if result.synced {
                    self.settings = self
                        .file
                        .load(self.settings.notes_dir.clone(), &self.home)?;
                    self.baseline = self.settings.clone();
                    if self.focus != Focus::Editor {
                        self.session = Session::default();
                    }
                    self.refresh()?;
                }
                if let Some(key) = result.recovery {
                    self.recovery_key = Some(key);
                    self.start_prompt(PromptKind::ConfirmRecovery);
                } else if result.unlocked
                    && !was_unlocked
                    && device.is_none()
                    && after_login.is_none()
                {
                    self.sync_now()?;
                }
            }
            Err(e) => {
                if device.is_none() {
                    self.sync_unlocked = false;
                }
                self.status = if device.as_ref().is_some_and(|cancel| cancel.is_cancelled()) {
                    "Device sign-in cancelled".into()
                } else {
                    e.to_string()
                };
            }
        }
        if let Some(command) = after_login {
            self.sync_start(command)?;
        }
        Ok(())
    }
    fn folder_arg(&self, arg: &str) -> io::Result<PathBuf> {
        if arg.is_empty() {
            return Err(io::Error::other(
                "Specify a folder path or its sidebar number",
            ));
        }
        if let Ok(index) = arg.parse::<usize>() {
            if index == 0 {
                return Err(io::Error::other("Folder numbers start at 1"));
            }
            return self
                .settings
                .note_folders
                .get(index.saturating_sub(1))
                .cloned()
                .ok_or_else(|| io::Error::other("Folder number out of range"));
        }
        let expanded = if let Some(suffix) = arg.strip_prefix("~/") {
            self.home.join(suffix)
        } else {
            PathBuf::from(arg)
        };
        Ok(expanded)
    }
    fn save_as(&mut self, arg: &str) -> io::Result<()> {
        if arg.is_empty() {
            return Err(io::Error::other("Usage: :saveas PATH (must not exist)"));
        }
        let path = if let Some(suffix) = arg.strip_prefix("~/") {
            self.home.join(suffix)
        } else {
            PathBuf::from(arg)
        };
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        check_notes_folder(parent)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(self.session.text.as_bytes())?;
        file.sync_all()?;
        let path = fs::canonicalize(path)?;
        self.session = Session::default();
        self.open(&path)?;
        self.refresh()?;
        self.status = format!("Saved a new copy: {}", path.display());
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(cancel) = &self.device_cancel {
            cancel.cancel();
        }
    }
}

fn session_only_login() -> bool {
    let ssh = std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    let graphical = !cfg!(target_os = "linux")
        || std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    ssh || !graphical
}

fn valid_color(value: &str) -> bool {
    value
        .strip_prefix("preset-")
        .and_then(|v| v.parse::<u8>().ok())
        .is_some_and(|v| v < 12)
        || (value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|c| c.is_ascii_hexdigit()))
}
