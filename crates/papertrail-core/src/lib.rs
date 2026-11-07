// SPDX-License-Identifier: GPL-3.0-or-later
//! UI-independent behavior shared by desktop and future command-line/mobile clients.
pub mod colors;
pub mod note;
pub mod session;
pub mod settings;

pub use note::Note;
pub use session::{Session, Store};
pub use settings::Settings;
