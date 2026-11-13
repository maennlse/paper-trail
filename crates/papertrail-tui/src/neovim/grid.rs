// SPDX-License-Identifier: GPL-3.0-or-later
//! Neovim's ext_linegrid UI, committed only at redraw flush boundaries.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use rmpv::Value;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Cell {
    text: String,
    highlight: u64,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            highlight: 0,
        }
    }
}
#[derive(Default)]
pub struct Grid {
    width: usize,
    height: usize,
    pending: Vec<Cell>,
    shown: Vec<Cell>,
    shown_width: usize,
    shown_height: usize,
    pending_cursor: (usize, usize),
    highlights: BTreeMap<u64, Style>,
    default: Style,
    pub cursor: (usize, usize),
    pub mode: String,
}
fn number(values: &[Value], index: usize) -> Option<usize> {
    usize::try_from(values.get(index)?.as_u64()?).ok()
}
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .as_map()?
        .iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, v)| v)
}
fn color(value: Option<&Value>) -> Option<Color> {
    let rgb = value?.as_u64()?;
    (rgb <= 0xffffff).then_some(Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
}
impl Grid {
    pub fn redraw(&mut self, events: &[Value]) {
        for event in events {
            let Some(event) = event.as_array() else {
                continue;
            };
            let Some(name) = event.first().and_then(Value::as_str) else {
                continue;
            };
            if name == "flush" {
                self.shown.clone_from(&self.pending);
                self.shown_width = self.width;
                self.shown_height = self.height;
                self.cursor = self.pending_cursor;
                continue;
            }
            for args in &event[1..] {
                if let Some(args) = args.as_array() {
                    self.event(name, args);
                }
            }
        }
    }
    fn event(&mut self, name: &str, args: &[Value]) {
        match name {
            "default_colors_set" => {
                self.default = Style::default();
                if let Some(c) = color(args.first()) {
                    self.default = self.default.fg(c);
                }
                if let Some(c) = color(args.get(1)) {
                    self.default = self.default.bg(c);
                }
            }
            "hl_attr_define" => {
                let Some(id) = args.first().and_then(Value::as_u64) else {
                    return;
                };
                let Some(attributes) = args.get(1) else {
                    return;
                };
                let mut style = Style::default();
                if let Some(c) = color(field(attributes, "foreground")) {
                    style = style.fg(c);
                }
                if let Some(c) = color(field(attributes, "background")) {
                    style = style.bg(c);
                }
                for (key, modifier) in [
                    ("bold", Modifier::BOLD),
                    ("italic", Modifier::ITALIC),
                    ("underline", Modifier::UNDERLINED),
                    ("undercurl", Modifier::UNDERLINED),
                    ("reverse", Modifier::REVERSED),
                    ("strikethrough", Modifier::CROSSED_OUT),
                ] {
                    if field(attributes, key).and_then(Value::as_bool) == Some(true) {
                        style = style.add_modifier(modifier);
                    }
                }
                self.highlights.insert(id, style);
            }
            "mode_change" => {
                if let Some(mode) = args.first().and_then(Value::as_str) {
                    self.mode = mode.into();
                }
            }
            "grid_resize" if args.first().and_then(Value::as_u64) == Some(1) => {
                let (Some(width), Some(height)) = (number(args, 1), number(args, 2)) else {
                    return;
                };
                if width == 0 || height == 0 || width.saturating_mul(height) > 1_000_000 {
                    return;
                }
                self.width = width;
                self.height = height;
                self.pending = vec![Cell::default(); width * height];
            }
            "grid_clear" if args.first().and_then(Value::as_u64) == Some(1) => {
                self.pending.fill(Cell::default())
            }
            "grid_cursor_goto" if args.first().and_then(Value::as_u64) == Some(1) => {
                if let (Some(row), Some(col)) = (number(args, 1), number(args, 2)) {
                    self.pending_cursor = (row, col);
                }
            }
            "grid_line" if args.first().and_then(Value::as_u64) == Some(1) => {
                let (Some(row), Some(mut col), Some(cells)) = (
                    number(args, 1),
                    number(args, 2),
                    args.get(3).and_then(Value::as_array),
                ) else {
                    return;
                };
                if row >= self.height {
                    return;
                }
                let mut highlight = 0;
                for item in cells {
                    let Some(item) = item.as_array() else {
                        continue;
                    };
                    let Some(text) = item.first().and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(id) = item.get(1).and_then(Value::as_u64) {
                        highlight = id;
                    }
                    let repeat = number(item, 2).unwrap_or(1).min(self.width);
                    for _ in 0..repeat {
                        if col >= self.width {
                            break;
                        }
                        self.pending[row * self.width + col] = Cell {
                            text: text.into(),
                            highlight,
                        };
                        col += 1;
                    }
                }
            }
            "grid_scroll" if args.first().and_then(Value::as_u64) == Some(1) => {
                let (Some(top), Some(bottom), Some(left), Some(right), Some(rows), Some(cols)) = (
                    number(args, 1),
                    number(args, 2),
                    number(args, 3),
                    number(args, 4),
                    args.get(5).and_then(Value::as_i64),
                    args.get(6).and_then(Value::as_i64),
                ) else {
                    return;
                };
                if top > bottom || bottom > self.height || left > right || right > self.width {
                    return;
                }
                let old = self.pending.clone();
                for row in top..bottom {
                    for col in left..right {
                        let source = (row as i64)
                            .checked_add(rows)
                            .zip((col as i64).checked_add(cols));
                        self.pending[row * self.width + col] = match source {
                            Some((r, c))
                                if (top as i64..bottom as i64).contains(&r)
                                    && (left as i64..right as i64).contains(&c) =>
                            {
                                old[r as usize * self.width + c as usize].clone()
                            }
                            _ => Cell::default(),
                        };
                    }
                }
            }
            _ => (),
        }
    }
    pub fn render(&self, area: Rect, buffer: &mut Buffer, focused: bool) {
        buffer.set_style(area, self.default);
        for row in 0..self.shown_height.min(usize::from(area.height)) {
            for col in 0..self.shown_width.min(usize::from(area.width)) {
                let Some(cell) = self.shown.get(row * self.shown_width + col) else {
                    continue;
                };
                let mut style = self.default.patch(
                    self.highlights
                        .get(&cell.highlight)
                        .copied()
                        .unwrap_or_default(),
                );
                if focused && self.cursor == (row, col) {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let x = area.x + col as u16;
                let y = area.y + row as u16;
                if !cell.text.is_empty() {
                    buffer.set_stringn(x, y, &cell.text, usize::from(area.right() - x), style);
                }
            }
        }
    }
}
