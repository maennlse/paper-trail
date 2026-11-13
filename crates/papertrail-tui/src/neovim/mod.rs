// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional real Neovim child. Paper Trail alone writes its managed note buffer.
mod grid;
mod rpc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect};
use rmpv::Value;
use rpc::{Message, Rpc};
use std::{
    io,
    path::{Path, PathBuf},
};

pub struct WriteRequest {
    pub id: u64,
    pub buffer: u64,
    pub text: String,
}
pub enum Event {
    Text(String),
    Write(WriteRequest),
}
pub struct Update {
    pub events: Vec<Event>,
    pub closed: bool,
}
pub struct Neovim {
    rpc: Rpc,
    channel: u64,
    pub grid: grid::Grid,
    pub path: Option<PathBuf>,
    buffer: u64,
    lines: Vec<String>,
    pub tick: u64,
    size: (u16, u16),
}
fn error(message: &str) -> io::Error {
    io::Error::other(message)
}
fn lines(text: &str) -> Value {
    Value::Array(text.split('\n').map(Value::from).collect())
}
fn text(value: &Value) -> Option<String> {
    Some(
        value
            .as_array()?
            .iter()
            .map(|line| line.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?
            .join("\n"),
    )
}
fn handle(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        if let Value::Ext(0, bytes) = value {
            rmpv::decode::read_value(&mut bytes.as_slice())
                .ok()
                .and_then(|value| value.as_u64())
        } else {
            None
        }
    })
}
fn api_channel(info: &Value) -> io::Result<u64> {
    let fields = info
        .as_array()
        .ok_or_else(|| error("Invalid Neovim API information"))?;
    let version = fields
        .get(1)
        .and_then(Value::as_map)
        .and_then(|map| map.iter().find(|(key, _)| key.as_str() == Some("version")))
        .and_then(|(_, value)| value.as_map())
        .ok_or_else(|| error("Neovim 0.9+ is required"))?;
    let number = |name| {
        version
            .iter()
            .find(|(key, _)| key.as_str() == Some(name))
            .and_then(|(_, value)| value.as_u64())
    };
    let (Some(major), Some(minor)) = (number("major"), number("minor")) else {
        return Err(error("Neovim 0.9+ is required"));
    };
    if major == 0 && minor < 9 {
        return Err(error(
            "Neovim 0.9+ is required; use :set editor builtin until it is upgraded",
        ));
    }
    fields
        .first()
        .and_then(Value::as_u64)
        .ok_or_else(|| error("Invalid Neovim channel"))
}
impl Neovim {
    #[cfg(not(test))]
    pub fn start(clean: bool, path: Option<&Path>, contents: &str) -> io::Result<Self> {
        Self::start_with(Rpc::spawn(clean, None)?, path, contents)
    }
    #[cfg(test)]
    pub fn start_isolated(
        clean: bool,
        path: Option<&Path>,
        contents: &str,
        home: &Path,
    ) -> io::Result<Self> {
        Self::start_with(Rpc::spawn(clean, Some(home))?, path, contents)
    }
    fn start_with(mut rpc: Rpc, path: Option<&Path>, contents: &str) -> io::Result<Self> {
        let info = rpc.call("nvim_get_api_info", vec![])?;
        let channel = api_channel(&info)?;
        rpc.call(
            "nvim_ui_attach",
            vec![
                80.into(),
                24.into(),
                Value::Map(vec![
                    ("rgb".into(), true.into()),
                    ("ext_linegrid".into(), true.into()),
                ]),
            ],
        )?;
        let mut result = Self {
            rpc,
            channel,
            grid: grid::Grid::default(),
            path: None,
            buffer: 0,
            lines: vec![],
            tick: 0,
            size: (80, 24),
        };
        result.load_with_channel(channel, path, contents)?;
        Ok(result)
    }
    fn load_with_channel(
        &mut self,
        channel: u64,
        path: Option<&Path>,
        contents: &str,
    ) -> io::Result<()> {
        let name = path
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let buffer = self
            .rpc
            .call(
                "nvim_exec_lua",
                vec![
                    include_str!("bridge.lua").into(),
                    Value::Array(vec![channel.into(), name.into(), lines(contents)]),
                ],
            )?
            .as_u64()
            .ok_or_else(|| error("Invalid Neovim note buffer"))?;
        self.buffer = buffer;
        self.path = path.map(Path::to_path_buf);
        self.lines = contents.split('\n').map(str::to_owned).collect();
        self.tick = 0;
        self.rpc.call(
            "nvim_buf_attach",
            vec![buffer.into(), true.into(), Value::Map(vec![])],
        )?;
        Ok(())
    }
    pub fn load(&mut self, path: Option<&Path>, contents: &str) -> io::Result<()> {
        self.load_with_channel(self.channel, path, contents)
    }
    pub fn input(&mut self, keys: &str) -> io::Result<()> {
        self.rpc.notify("nvim_input", vec![keys.into()])
    }
    pub fn key(&mut self, key: KeyEvent) -> io::Result<()> {
        if let Some(keys) = key_notation(key) {
            self.input(&keys)?;
        }
        Ok(())
    }
    pub fn paste(&mut self, contents: &str) -> io::Result<()> {
        self.rpc.notify(
            "nvim_paste",
            vec![contents.into(), false.into(), (-1).into()],
        )
    }
    pub fn replace(&mut self, contents: &str) -> io::Result<()> {
        self.rpc.notify(
            "nvim_buf_set_lines",
            vec![
                self.buffer.into(),
                0.into(),
                (-1).into(),
                false.into(),
                lines(contents),
            ],
        )
    }
    pub fn snapshot(&mut self) -> io::Result<String> {
        // Leaving the pane/app command prompt cancels partial commands first, so
        // a deferred API request cannot wait forever for another editing key.
        self.input("<Esc>")?;
        let result = self.rpc.call("nvim_exec_lua", vec!["local b=...; return {vim.api.nvim_buf_get_changedtick(b),vim.api.nvim_buf_get_lines(b,0,-1,false)}".into(), Value::Array(vec![self.buffer.into()])])?;
        let values = result
            .as_array()
            .ok_or_else(|| error("Invalid Neovim buffer snapshot"))?;
        self.tick = values
            .first()
            .and_then(Value::as_u64)
            .ok_or_else(|| error("Invalid Neovim changed tick"))?;
        let contents = values
            .get(1)
            .and_then(text)
            .ok_or_else(|| error("Invalid Neovim buffer text"))?;
        self.lines = contents.split('\n').map(str::to_owned).collect();
        Ok(contents)
    }
    pub fn mark_saved(&mut self) {
        let _ = self.rpc.notify("nvim_exec_lua", vec!["local b,t=...; if vim.api.nvim_buf_is_valid(b) and vim.api.nvim_buf_get_changedtick(b)==t then vim.bo[b].modified=false end".into(), Value::Array(vec![self.buffer.into(), self.tick.into()])]);
    }
    pub fn mark_dirty(&mut self) -> io::Result<()> {
        self.rpc.notify(
            "nvim_buf_set_option",
            vec![self.buffer.into(), "modified".into(), true.into()],
        )
    }
    pub fn reply_write(&mut self, id: u64, result: io::Result<()>) -> io::Result<()> {
        self.rpc.reply(id, result.map(|()| true.into()))
    }
    pub fn owns_buffer(&self, buffer: u64) -> bool {
        buffer == self.buffer
    }
    pub fn poll(&mut self) -> Update {
        let mut update = Update {
            events: vec![],
            closed: false,
        };
        for message in self.rpc.poll() {
            match message {
                Message::Notification(name, args) if name == "redraw" => self.grid.redraw(&args),
                Message::Notification(name, args) if name == "nvim_buf_lines_event" => {
                    if args.first().and_then(handle) != Some(self.buffer) {
                        continue;
                    }
                    // Inccommand previews (nil ticks) are not real edits. A
                    // snapshot also supersedes older queued buffer events.
                    let Some(tick) = args.get(1).and_then(Value::as_u64) else {
                        continue;
                    };
                    if tick <= self.tick {
                        continue;
                    }
                    let Some(first) = args
                        .get(2)
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                    else {
                        continue;
                    };
                    let Some(last) = args.get(3).and_then(Value::as_i64) else {
                        continue;
                    };
                    let Some(replacement) = args.get(4).and_then(Value::as_array).and_then(|v| {
                        v.iter()
                            .map(|s| s.as_str().map(str::to_owned))
                            .collect::<Option<Vec<_>>>()
                    }) else {
                        continue;
                    };
                    let last = if last == -1 {
                        self.lines.len()
                    } else {
                        usize::try_from(last).unwrap_or(usize::MAX)
                    };
                    if first > last || last > self.lines.len() {
                        continue;
                    }
                    self.lines.splice(first..last, replacement);
                    if args.get(5).and_then(Value::as_bool) != Some(true) {
                        self.tick = tick;
                        update.events.push(Event::Text(self.lines.join("\n")));
                    }
                }
                Message::Request(id, name, args) if name == "papertrail_write" => {
                    if let (Some(buffer), Some(_tick), Some(contents)) = (
                        args.first().and_then(Value::as_u64),
                        args.get(1).and_then(Value::as_u64),
                        args.get(2).and_then(text),
                    ) {
                        update.events.push(Event::Write(WriteRequest {
                            id,
                            buffer,
                            text: contents,
                        }));
                    } else {
                        let _ = self.rpc.reply(id, Err(error("Invalid note write request")));
                    }
                }
                Message::Request(id, _, _) => {
                    let _ = self
                        .rpc
                        .reply(id, Err(error("Unsupported Paper Trail RPC request")));
                }
                Message::Closed => update.closed = true,
                _ => (),
            }
        }
        update
    }
    pub fn resize(&mut self, width: u16, height: u16) {
        let size = (width.max(1), height.max(1));
        if self.size != size {
            self.size = size;
            let _ = self
                .rpc
                .notify("nvim_ui_try_resize", vec![size.0.into(), size.1.into()]);
        }
    }
    pub fn render(&self, area: Rect, buffer: &mut Buffer, focused: bool) {
        self.grid.render(area, buffer, focused);
    }
    pub fn mouse(
        &mut self,
        kind: MouseEventKind,
        row: u16,
        col: u16,
        modifiers: KeyModifiers,
    ) -> io::Result<()> {
        let (button, action) = match kind {
            MouseEventKind::ScrollUp => ("wheel", "up"),
            MouseEventKind::ScrollDown => ("wheel", "down"),
            MouseEventKind::Down(b) => (mouse_button(b), "press"),
            MouseEventKind::Up(b) => (mouse_button(b), "release"),
            MouseEventKind::Drag(b) => (mouse_button(b), "drag"),
            _ => return Ok(()),
        };
        self.rpc.notify(
            "nvim_input_mouse",
            vec![
                button.into(),
                action.into(),
                modifier_notation(modifiers).into(),
                0.into(),
                row.into(),
                col.into(),
            ],
        )
    }
}
fn mouse_button(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}
pub fn key_notation(key: KeyEvent) -> Option<String> {
    let name = match key.code {
        KeyCode::Char('<')
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            return Some("<LT>".into())
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            return Some(c.to_string())
        }
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char('<') => "LT".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "CR".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Backspace => "BS".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => return Some("<S-Tab>".into()),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Delete => "Del".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::F(n) => format!("F{n}"),
        _ => return None,
    };
    Some(format!("<{}{name}>", modifier_notation(key.modifiers)))
}
fn modifier_notation(flags: KeyModifiers) -> String {
    let mut modifiers = String::new();
    for (flag, prefix) in [
        (KeyModifiers::CONTROL, "C-"),
        (KeyModifiers::ALT, "A-"),
        (KeyModifiers::SHIFT, "S-"),
        (KeyModifiers::SUPER, "D-"),
    ] {
        if flags.contains(flag) {
            modifiers.push_str(prefix);
        }
    }
    modifiers
}

#[cfg(test)]
pub(crate) mod tests;
