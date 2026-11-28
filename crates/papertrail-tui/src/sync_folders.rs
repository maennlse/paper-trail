// SPDX-License-Identifier: GPL-3.0-or-later
//! Device-local selection drafts, plus separately confirmed server operations.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use papertrail_core::Settings;
use papertrail_storage::startup::check_notes_folder;
use papertrail_sync::{control::Service, folders::FolderChoice, same_directory, Config};
use ratatui::{
    layout::{Position, Rect},
    widgets::ListState,
};
use std::{collections::BTreeSet, fs, io};

pub enum Action {
    None,
    Save,
    Cancel,
    Delete(String),
    Resync(String),
}

pub struct Confirmation {
    pub id: String,
    pub name: String,
    pub resync: bool,
    pub accept: bool,
    pub scroll: u16,
}

pub struct Picker {
    pub choices: Vec<FolderChoice>,
    pub confirmation: Option<Confirmation>,
    pub busy: bool,
    pub message: Option<String>,
    pub state: ListState,
    pub area: Rect,
    pub details: bool,
    pub details_scroll: u16,
    colors: Vec<String>,
    restartable: BTreeSet<String>,
    published: BTreeSet<String>,
    previous: Config,
}
impl Picker {
    pub fn open(service: &Service, settings: &Settings) -> io::Result<Self> {
        let previous = service
            .saved_config()?
            .ok_or_else(|| io::Error::other("Configure Sync first with :sync-configure SERVER"))?;
        let mut choices = service.folder_choices(settings)?;
        let snapshot = service.snapshot()?;
        let catalog = snapshot
            .as_ref()
            .and_then(|state| state.catalog.as_ref().or(state.catalog_draft.as_ref()));
        let mut colors: Vec<String> = choices
            .iter()
            .map(|choice| {
                choice
                    .path
                    .as_ref()
                    .and_then(|path| {
                        settings.folder_colors.get(path).or_else(|| {
                            settings
                                .folder_colors
                                .iter()
                                .find(|(alias, _)| same_directory(alias, path))
                                .map(|(_, color)| color)
                        })
                    })
                    .or_else(|| {
                        choice.id.as_ref().and_then(|id| {
                            catalog
                                .and_then(|catalog| catalog.folders.get(id))
                                .map(|folder| &folder.color)
                        })
                    })
                    .cloned()
                    .unwrap_or_else(|| "preset-0".into())
            })
            .collect();
        let mut restartable = BTreeSet::new();
        let mut published = BTreeSet::new();
        if let Some(state) = &snapshot {
            if let Some(catalog) = &state.catalog {
                for (id, data) in &catalog.folders {
                    if catalog.deleted_folders.contains(id) {
                        let Some(binding) = state.bindings.get(id).filter(|b| {
                            b.path.is_dir()
                                && settings
                                    .note_folders
                                    .iter()
                                    .any(|p| same_directory(p, &b.path))
                        }) else {
                            continue;
                        };
                        choices.push(FolderChoice {
                            id: Some(id.clone()),
                            path: Some(binding.path.clone()),
                            name: data.name.clone(),
                            selected: false,
                            conflict: false,
                            stored_path: None,
                        });
                        colors.push(data.color.clone());
                        restartable.insert(id.clone());
                    } else {
                        published.insert(id.clone());
                    }
                }
            }
        }
        Ok(Self {
            choices,
            restartable,
            published,
            confirmation: None,
            busy: false,
            message: None,
            state: ListState::default().with_selected(Some(0)),
            area: Rect::default(),
            details: false,
            details_scroll: 0,
            colors,
            previous,
        })
    }
    pub fn color(&self, index: usize) -> &str {
        self.colors
            .get(index)
            .map(String::as_str)
            .unwrap_or("preset-0")
    }
    pub fn visible_choices(&self) -> &[FolderChoice] {
        &self.choices
    }
    pub fn local_only(&self, index: usize) -> bool {
        self.choices
            .get(index)
            .and_then(|c| c.id.as_ref())
            .is_some_and(|id| self.restartable.contains(id))
    }
    pub fn check_current(&self, service: &Service) -> io::Result<()> {
        if service.saved_config()?.as_ref() != Some(&self.previous) {
            return Err(io::Error::other("Sync configuration changed while selecting folders. Esc to cancel, then reopen :sync-folders"));
        }
        Ok(())
    }
    /// Reconcile remote operations without discarding the user's other toggles.
    pub fn refresh(
        &mut self,
        service: &Service,
        settings: &Settings,
        resynced: bool,
    ) -> io::Result<()> {
        let restart_path = resynced
            .then(|| {
                self.state
                    .selected()
                    .and_then(|i| self.visible_choices().get(i))
                    .and_then(|c| c.path.clone())
            })
            .flatten();
        let mut fresh = Self::open(service, settings)?;
        for choice in &mut fresh.choices {
            if choice
                .id
                .as_ref()
                .is_some_and(|id| fresh.restartable.contains(id))
            {
                continue;
            }
            if let Some(old) = self.choices.iter().find(|old| {
                (choice.id.is_some() && old.id == choice.id)
                    || (old.id.is_none()
                        && old
                            .path
                            .as_ref()
                            .zip(choice.path.as_ref())
                            .is_some_and(|(a, b)| same_directory(a, b)))
            }) {
                choice.selected = old.selected;
            }
        }
        if !resynced {
            fresh.previous = self.previous.clone();
        }
        fresh.state.select(
            self.state
                .selected()
                .map(|i| i.min(fresh.visible_choices().len().saturating_sub(1))),
        );
        if let Some(path) = restart_path {
            fresh.state.select(
                fresh
                    .choices
                    .iter()
                    .position(|c| c.path.as_ref().is_some_and(|p| same_directory(p, &path))),
            );
        }
        fresh.area = self.area;
        fresh.message = self.message.clone();
        *self = fresh;
        Ok(())
    }
    fn request_operation(&mut self, resync: bool) {
        let Some(choice) = self
            .state
            .selected()
            .and_then(|i| self.visible_choices().get(i))
        else {
            return;
        };
        let Some(id) = &choice.id else {
            self.message = Some("This folder has not been published to the server".into());
            return;
        };
        if resync && !self.restartable.contains(id) {
            self.message =
                Some("This folder already has a sync identity; use Space to select it".into());
            return;
        }
        if !resync && self.restartable.contains(id) {
            self.message = Some("This folder is local only; use s to Sync as New Folder".into());
            return;
        }
        if !resync && !self.published.contains(id) {
            self.message = Some("This folder has not been published to the server".into());
            return;
        }
        self.confirmation = Some(Confirmation {
            id: id.clone(),
            name: choice.name.clone(),
            resync,
            accept: false,
            scroll: 0,
        });
    }
    pub fn description(&self) -> String {
        let Some(choice) = self
            .state
            .selected()
            .and_then(|index| self.visible_choices().get(index))
        else {
            return String::new();
        };
        let path = choice
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| {
                format!(
                    "Automatic download under {}",
                    self.previous
                        .download_root()
                        .map(|root| root.display().to_string())
                        .unwrap_or_default()
                )
            });
        if choice
            .id
            .as_ref()
            .is_some_and(|id| self.restartable.contains(id))
        {
            return format!("{} · Local only\nLocal: {path}\nUse s to Sync as New Folder. The deleted identity and history cannot be restored.", choice.name);
        }
        format!(
            "{}\nLocal: {path}\nID: {}{}{}",
            choice.name,
            choice
                .id
                .as_deref()
                .unwrap_or("Local folder · registered on next sync"),
            if choice.conflict {
                "\nMetadata conflict: use :sync-accept-remote ID after reviewing the remote name/color."
            } else {
                ""
            },
            choice
                .stored_path
                .as_ref()
                .map(|p| format!(
                    "\nCurrent directory: {} (mapping change pending)",
                    p.display()
                ))
                .unwrap_or_default()
        )
    }
    fn move_by(&mut self, delta: isize) {
        self.state.select(Some(
            self.state
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(self.visible_choices().len().saturating_sub(1)),
        ));
    }
    fn toggle(&mut self) {
        if self.state.selected().is_some_and(|i| self.local_only(i)) {
            self.message =
                Some("Use s to Sync as New Folder; the deleted identity cannot be reused".into());
            return;
        }
        if let Some(choice) = self
            .state
            .selected()
            .and_then(|index| self.choices.get_mut(index))
        {
            choice.selected = !choice.selected;
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.busy {
            // Closing is safe: the confirmed remote operation still completes.
            return if key.code == KeyCode::Esc
                || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'))
            {
                Action::Cancel
            } else {
                Action::None
            };
        }
        if let Some(confirm) = &mut self.confirmation {
            match key.code {
                KeyCode::Esc => self.confirmation = None,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.confirmation = None
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    confirm.accept = !confirm.accept
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    confirm.scroll = confirm.scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    confirm.scroll = confirm.scroll.saturating_sub(1)
                }
                KeyCode::PageDown => confirm.scroll = confirm.scroll.saturating_add(5),
                KeyCode::PageUp => confirm.scroll = confirm.scroll.saturating_sub(5),
                KeyCode::Enter => {
                    let confirm = self.confirmation.take().unwrap();
                    if confirm.accept {
                        return if confirm.resync {
                            Action::Resync(confirm.id)
                        } else {
                            Action::Delete(confirm.id)
                        };
                    }
                }
                _ => (),
            }
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return if key.code == KeyCode::Char('c') {
                Action::Cancel
            } else {
                Action::None
            };
        }
        if self.details {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i') => self.details = false,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.details_scroll = self.details_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.details_scroll = self.details_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => self.details_scroll = self.details_scroll.saturating_add(5),
                KeyCode::PageUp => self.details_scroll = self.details_scroll.saturating_sub(5),
                KeyCode::Home => self.details_scroll = 0,
                _ => (),
            }
            return Action::None;
        }
        match key.code {
            KeyCode::Esc => return Action::Cancel,
            KeyCode::Enter => return Action::Save,
            KeyCode::Char('d') | KeyCode::Delete => self.request_operation(false),
            KeyCode::Char('s') => self.request_operation(true),
            KeyCode::Char(' ') => self.toggle(),
            KeyCode::Char('i') => {
                self.details = true;
                self.details_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::PageDown => self.move_by(self.area.height.max(1) as isize),
            KeyCode::PageUp => self.move_by(-(self.area.height.max(1) as isize)),
            KeyCode::Home => self.state.select(Some(0)),
            KeyCode::End => self
                .state
                .select(Some(self.visible_choices().len().saturating_sub(1))),
            _ => (),
        }
        Action::None
    }
    pub fn mouse(&mut self, event: MouseEvent) {
        if self.busy {
            return;
        }
        if let Some(confirm) = &mut self.confirmation {
            match event.kind {
                MouseEventKind::ScrollDown => confirm.scroll = confirm.scroll.saturating_add(3),
                MouseEventKind::ScrollUp => confirm.scroll = confirm.scroll.saturating_sub(3),
                _ => (),
            }
            return;
        }
        if !self.area.contains(Position::new(event.column, event.row)) {
            return;
        }
        if self.details {
            match event.kind {
                MouseEventKind::ScrollDown => {
                    self.details_scroll = self.details_scroll.saturating_add(3)
                }
                MouseEventKind::ScrollUp => {
                    self.details_scroll = self.details_scroll.saturating_sub(3)
                }
                _ => (),
            }
            return;
        }
        match event.kind {
            MouseEventKind::ScrollDown => self.move_by(3),
            MouseEventKind::ScrollUp => self.move_by(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                let index = self.state.offset() + usize::from(event.row - self.area.y);
                if index < self.visible_choices().len() {
                    self.state.select(Some(index));
                    self.toggle();
                }
            }
            _ => (),
        }
    }
    pub fn save(&self, service: &Service) -> io::Result<()> {
        self.check_current(service)?;
        let mut config = self.previous.clone();
        // Keep a valid download-only configuration even if every upload is unchecked.
        config.download_dir = Some(config.download_root()?);
        for choice in &self.choices {
            let selected = choice.selected
                && !choice
                    .id
                    .as_ref()
                    .is_some_and(|id| self.restartable.contains(id));
            if let Some(id) = &choice.id {
                if selected {
                    config.selected_folders.insert(id.clone());
                    config.excluded_folders.remove(id);
                } else {
                    config.selected_folders.remove(id);
                    config.excluded_folders.insert(id.clone());
                }
            }
            if let Some(path) = &choice.path {
                if !selected {
                    config
                        .folders
                        .retain(|folder| !same_directory(folder, path));
                } else if !config
                    .folders
                    .iter()
                    .any(|folder| same_directory(folder, path))
                {
                    let path = if choice.id.is_none() {
                        let path = fs::canonicalize(path)?;
                        check_notes_folder(&path)?;
                        path
                    } else {
                        path.clone()
                    };
                    config.folders.push(path);
                }
            }
        }
        service.configure(config)
    }
}
