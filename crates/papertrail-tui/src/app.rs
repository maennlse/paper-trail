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
}
pub struct Prompt {
    pub kind: PromptKind,
    pub text: String,
    pub completion: crate::completion::Completion,
}

pub struct App {
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
        self.quit = true;
        Ok(())
    }
    pub fn key(&mut self, key: KeyEvent) -> io::Result<()> {
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

fn valid_color(value: &str) -> bool {
    value
        .strip_prefix("preset-")
        .and_then(|v| v.parse::<u8>().ok())
        .is_some_and(|v| v < 12)
        || (value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|c| c.is_ascii_hexdigit()))
}
