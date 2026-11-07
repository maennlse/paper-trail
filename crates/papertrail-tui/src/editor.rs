// SPDX-License-Identifier: GPL-3.0-or-later
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::{CursorMove, TextArea};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
}

#[derive(Default)]
pub struct Editor {
    pub area: TextArea<'static>,
    pub mode: Mode,
    pub pending: Option<char>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    top: u16,
    left: u16,
}

struct Change {
    start: usize,
    removed: String,
    inserted: String,
    before: (usize, usize),
    after: (usize, usize),
}

impl Editor {
    pub fn set_text(&mut self, text: &str) {
        self.area = TextArea::new(text.split('\n').map(str::to_owned).collect());
        self.area.set_tab_length(4);
        self.area.set_max_histories(0);
        self.mode = Mode::Normal;
        self.pending = None;
        self.undo.clear();
        self.redo.clear();
        self.top = 0;
        self.left = 0;
    }
    pub fn text(&self) -> String {
        self.area.lines().join("\n")
    }

    /// Mirror tui-textarea 0.7's viewport so pointer coordinates use the rendered
    /// scroll position, rather than assuming the first line/column is visible.
    pub fn track_view(&mut self, width: u16, height: u16) {
        fn follow(top: u16, cursor: usize, length: u16) -> u16 {
            let cursor = cursor.min(usize::from(u16::MAX)) as u16;
            if cursor < top {
                cursor
            } else if usize::from(top) + usize::from(length) <= usize::from(cursor) {
                cursor.saturating_add(1).saturating_sub(length)
            } else {
                top
            }
        }
        self.top = follow(self.top, self.area.cursor().0, height);
        let mut col = self.area.cursor().1;
        if self.area.line_number_style().is_some() {
            let gutter = self.gutter();
            col = if col <= gutter { col * 2 } else { col + gutter };
        }
        self.left = follow(self.left, col, width);
    }

    fn gutter(&self) -> usize {
        if self.area.line_number_style().is_some() {
            self.area.lines().len().to_string().len() + 2
        } else {
            0
        }
    }

    pub fn scroll(&mut self, rows: i16) {
        self.area.cancel_selection();
        if self.mode == Mode::Visual {
            self.mode = Mode::Normal;
        }
        self.top = self.top.saturating_add_signed(rows);
        self.area.scroll((rows, 0));
    }

    pub fn click(&mut self, y: u16, x: u16) {
        let row = (usize::from(self.top) + usize::from(y)).min(self.area.lines().len() - 1);
        let target = (usize::from(self.left) + usize::from(x)).saturating_sub(self.gutter());
        let mut width = 0;
        let mut column = 0;
        for c in self.area.lines()[row].chars() {
            let next = if c == '\t' {
                4 - width % 4
            } else {
                c.width().unwrap_or(0)
            };
            if width + next > target {
                break;
            }
            width += next;
            column += 1;
        }
        self.area.cancel_selection();
        self.pending = None;
        if self.mode == Mode::Visual {
            self.mode = Mode::Normal;
        }
        self.area.move_cursor(CursorMove::Jump(
            row.min(u16::MAX as usize) as u16,
            column.min(u16::MAX as usize) as u16,
        ));
    }
    pub fn key(&mut self, key: KeyEvent) {
        if self.mode != Mode::Insert {
            if key.code == KeyCode::Char('u') && key.modifiers.is_empty() {
                self.history(false);
                return;
            }
            if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.history(true);
                return;
            }
        }
        let before = self.text();
        let cursor = self.area.cursor();
        self.handle_key(key);
        self.record(&before, cursor);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            if self.mode == Mode::Insert && self.area.cursor().1 > 0 {
                self.area.move_cursor(CursorMove::Back);
            }
            self.mode = Mode::Normal;
            self.area.cancel_selection();
            self.pending = None;
            return;
        }
        if self.mode == Mode::Insert {
            self.area.input(key);
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('f') => {
                    for _ in 0..20 {
                        self.area.move_cursor(CursorMove::Down);
                    }
                }
                KeyCode::Char('b') => {
                    for _ in 0..20 {
                        self.area.move_cursor(CursorMove::Up);
                    }
                }
                _ => (),
            }
            return;
        }
        let pending = self.pending.take();
        match (pending, key.code) {
            (Some('g'), KeyCode::Char('g')) => self.area.move_cursor(CursorMove::Top),
            (Some('d'), KeyCode::Char('d')) => {
                self.area.move_cursor(CursorMove::Head);
                self.area.delete_line_by_end();
                // Join the now-empty line with the following line where possible.
                if self.area.lines().len() > 1 {
                    if self.area.cursor().0 + 1 == self.area.lines().len() {
                        self.area.delete_char();
                    } else {
                        self.area.delete_next_char();
                    }
                }
            }
            (Some('y'), KeyCode::Char('y')) => {
                self.area.move_cursor(CursorMove::Head);
                self.area.start_selection();
                self.area.move_cursor(CursorMove::End);
                self.area.copy();
                self.area.cancel_selection();
            }
            (_, KeyCode::Char('h') | KeyCode::Left) => self.area.move_cursor(CursorMove::Back),
            (_, KeyCode::Char('j') | KeyCode::Down) => self.area.move_cursor(CursorMove::Down),
            (_, KeyCode::Char('k') | KeyCode::Up) => self.area.move_cursor(CursorMove::Up),
            (_, KeyCode::Char('l') | KeyCode::Right) => self.area.move_cursor(CursorMove::Forward),
            (_, KeyCode::Char('w')) => self.area.move_cursor(CursorMove::WordForward),
            (_, KeyCode::Char('b')) => self.area.move_cursor(CursorMove::WordBack),
            (_, KeyCode::Char('0') | KeyCode::Home) => self.area.move_cursor(CursorMove::Head),
            (_, KeyCode::Char('$') | KeyCode::End) => self.area.move_cursor(CursorMove::End),
            (_, KeyCode::Char('G')) => self.area.move_cursor(CursorMove::Bottom),
            (_, KeyCode::Char('g' | 'd' | 'y')) if self.mode == Mode::Normal => {
                if let KeyCode::Char(c) = key.code {
                    self.pending = Some(c);
                }
            }
            (_, KeyCode::Char('i')) => self.mode = Mode::Insert,
            (_, KeyCode::Char('a')) => {
                self.area.move_cursor(CursorMove::Forward);
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('I')) => {
                self.area.move_cursor(CursorMove::Head);
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('A')) => {
                self.area.move_cursor(CursorMove::End);
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('o')) => {
                self.area.move_cursor(CursorMove::End);
                self.area.insert_newline();
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('O')) => {
                self.area.move_cursor(CursorMove::Head);
                self.area.insert_newline();
                self.area.move_cursor(CursorMove::Up);
                self.mode = Mode::Insert;
            }
            (_, KeyCode::Char('v')) => {
                self.area.start_selection();
                self.mode = Mode::Visual;
            }
            (_, KeyCode::Char('y')) if self.mode == Mode::Visual => {
                self.area.copy();
                self.area.cancel_selection();
                self.mode = Mode::Normal;
            }
            (_, KeyCode::Char('d' | 'x')) if self.mode == Mode::Visual => {
                self.area.cut();
                self.mode = Mode::Normal;
            }
            (_, KeyCode::Char('x') | KeyCode::Delete) => {
                self.area.delete_next_char();
            }
            (_, KeyCode::Char('p')) => {
                self.area.paste();
            }
            _ => (),
        }
    }
    pub fn find(&mut self, query: &str, forward: bool) -> bool {
        self.area.set_search_pattern(regex_escape(query)).is_ok()
            && if forward {
                self.area.search_forward(true)
            } else {
                self.area.search_back(true)
            }
    }

    pub fn paste_text(&mut self, text: &str) {
        let before = self.text();
        let cursor = self.area.cursor();
        self.area.insert_str(text);
        self.record(&before, cursor);
    }

    pub fn replace_text(&mut self, text: &str) {
        let before = self.text();
        let cursor = self.area.cursor();
        self.area.select_all();
        self.area.insert_str(text);
        self.record(&before, cursor);
    }

    fn record(&mut self, before: &str, cursor: (usize, usize)) {
        let after = self.text();
        if before == after {
            return;
        }
        let start: usize = before
            .chars()
            .zip(after.chars())
            .take_while(|(a, b)| a == b)
            .map(|(c, _)| c.len_utf8())
            .sum();
        let suffix: usize = before[start..]
            .chars()
            .rev()
            .zip(after[start..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(c, _)| c.len_utf8())
            .sum();
        self.undo.push(Change {
            start,
            removed: before[start..before.len() - suffix].to_owned(),
            inserted: after[start..after.len() - suffix].to_owned(),
            before: cursor,
            after: self.area.cursor(),
        });
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn history(&mut self, redo: bool) {
        let change = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        let Some(change) = change else {
            return;
        };
        let mut text = self.text();
        let (remove, insert, cursor) = if redo {
            (&change.removed, &change.inserted, change.after)
        } else {
            (&change.inserted, &change.removed, change.before)
        };
        text.replace_range(change.start..change.start + remove.len(), insert);
        let yank = self.area.yank_text();
        self.area = TextArea::new(text.split('\n').map(str::to_owned).collect());
        self.area.set_yank_text(yank);
        self.area.set_tab_length(4);
        self.area.set_max_histories(0);
        self.top = 0;
        self.left = 0;
        self.area.move_cursor(CursorMove::Jump(
            cursor.0.min(u16::MAX as usize) as u16,
            cursor.1.min(u16::MAX as usize) as u16,
        ));
        self.area.cancel_selection();
        self.mode = Mode::Normal;
        self.pending = None;
        if redo {
            self.undo.push(change);
        } else {
            self.redo.push(change);
        }
    }
}

fn regex_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
