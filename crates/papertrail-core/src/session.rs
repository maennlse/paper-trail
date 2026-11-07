// SPDX-License-Identifier: GPL-3.0-or-later
use crate::Note;
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

pub trait Store {
    fn load(&self, path: &Path) -> io::Result<Note>;
    fn save(&self, path: &Path, text: &str) -> io::Result<Note>;
    fn save_if_unchanged(&self, path: &Path, text: &str, _original: &Note) -> io::Result<Note> {
        self.save(path, text)
    }
}

/// Owns unsaved text independently of a widget and never drops it after a failed write.
#[derive(Default)]
pub struct Session {
    pub current: Option<Note>,
    pub text: String,
    dirty_since: Option<Instant>,
}

impl Session {
    pub const SAVE_DELAY: Duration = Duration::from_millis(350);
    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }
    pub fn edit(&mut self, text: String, now: Instant) {
        if self.current.is_some() {
            self.text = text;
            self.dirty_since = Some(now);
        }
    }
    pub fn save_due(&self, now: Instant) -> bool {
        self.dirty_since
            .is_some_and(|t| now.saturating_duration_since(t) >= Self::SAVE_DELAY)
    }
    pub fn flush(&mut self, store: &impl Store) -> io::Result<()> {
        if self.is_dirty() {
            if let Some(note) = &self.current {
                self.current = Some(store.save_if_unchanged(&note.path, &self.text, note)?);
            }
            self.dirty_since = None;
        }
        Ok(())
    }
    pub fn open(&mut self, store: &impl Store, path: &Path) -> io::Result<()> {
        self.flush(store)?;
        let note = store.load(path)?;
        self.text = note.body.clone().unwrap_or_default();
        self.current = Some(note);
        Ok(())
    }
    pub fn close(&mut self, store: &impl Store) -> io::Result<()> {
        self.flush(store)?;
        self.current = None;
        self.text.clear();
        Ok(())
    }
    pub fn discard_deleted(&mut self, path: &Path) {
        if self.current.as_ref().is_some_and(|n| n.path == path) {
            self.current = None;
            self.text.clear();
            self.dirty_since = None;
        }
    }
}
