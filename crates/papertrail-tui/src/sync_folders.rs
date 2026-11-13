// SPDX-License-Identifier: GPL-3.0-or-later
//! A device-local sync selection draft: nothing is persisted until Save.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use papertrail_core::Settings;
use papertrail_storage::startup::check_notes_folder;
use papertrail_sync::{control::Service, folders::FolderChoice, same_directory, Config};
use ratatui::{
    layout::{Position, Rect},
    widgets::ListState,
};
use std::{fs, io};

pub enum Action {
    None,
    Save,
    Cancel,
}

pub struct Picker {
    pub choices: Vec<FolderChoice>,
    pub state: ListState,
    pub area: Rect,
    pub details: bool,
    pub details_scroll: u16,
    colors: Vec<String>,
    previous: Config,
}
impl Picker {
    pub fn open(service: &Service, settings: &Settings) -> io::Result<Self> {
        let previous = service
            .saved_config()?
            .ok_or_else(|| io::Error::other("Configure Sync first with :sync-configure SERVER"))?;
        let choices = service.folder_choices(settings)?;
        let snapshot = service.snapshot()?;
        let catalog = snapshot
            .as_ref()
            .and_then(|state| state.catalog.as_ref().or(state.catalog_draft.as_ref()));
        let colors = choices
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
        Ok(Self {
            choices,
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
    pub fn description(&self) -> String {
        let Some(choice) = self
            .state
            .selected()
            .and_then(|index| self.choices.get(index))
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
        format!(
            "{}\nLocal: {path}\nID: {}{}",
            choice.name,
            choice
                .id
                .as_deref()
                .unwrap_or("Local folder · registered on next sync"),
            if choice.conflict {
                "\nMetadata conflict: use :sync-accept-remote ID after reviewing the remote name/color."
            } else {
                ""
            }
        )
    }
    fn move_by(&mut self, delta: isize) {
        self.state.select(Some(
            self.state
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(self.choices.len().saturating_sub(1)),
        ));
    }
    fn toggle(&mut self) {
        if let Some(choice) = self
            .state
            .selected()
            .and_then(|index| self.choices.get_mut(index))
        {
            choice.selected = !choice.selected;
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
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
                .select(Some(self.choices.len().saturating_sub(1))),
            _ => (),
        }
        Action::None
    }
    pub fn mouse(&mut self, event: MouseEvent) {
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
                if index < self.choices.len() {
                    self.state.select(Some(index));
                    self.toggle();
                }
            }
            _ => (),
        }
    }
    pub fn save(&self, service: &Service) -> io::Result<()> {
        if service.saved_config()?.as_ref() != Some(&self.previous) {
            return Err(io::Error::other("Sync configuration changed while selecting folders. Esc to cancel, then reopen :sync-folders"));
        }
        let mut config = self.previous.clone();
        // Keep a valid download-only configuration even if every upload is unchecked.
        config.download_dir = Some(config.download_root()?);
        for choice in &self.choices {
            if let Some(id) = &choice.id {
                if choice.selected {
                    config.selected_folders.insert(id.clone());
                    config.excluded_folders.remove(id);
                } else {
                    config.selected_folders.remove(id);
                    config.excluded_folders.insert(id.clone());
                }
            }
            if let Some(path) = &choice.path {
                if !choice.selected {
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
