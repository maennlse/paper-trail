// SPDX-License-Identifier: GPL-3.0-or-later
//! Private stdio MessagePack-RPC. No shell or authentication/vault secrets.
use rmpv::Value;
use std::{
    collections::VecDeque,
    io::{self, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

pub enum Message {
    Notification(String, Vec<Value>),
    Request(u64, String, Vec<Value>),
    Closed,
}

pub struct Rpc {
    child: Child,
    input: SyncSender<Value>,
    frames: Receiver<Value>,
    queued: VecDeque<Message>,
    next: u64,
}
fn error(message: &str) -> io::Error {
    io::Error::other(message)
}
impl Rpc {
    pub fn spawn(clean: bool, isolated_home: Option<&Path>) -> io::Result<Self> {
        Self::spawn_command(Command::new("nvim"), clean, isolated_home)
    }
    fn spawn_command(
        mut command: Command,
        clean: bool,
        isolated_home: Option<&Path>,
    ) -> io::Result<Self> {
        command.args(["--embed", "-i", "NONE"]);
        command.env_remove("NVIM").env_remove("NVIM_LISTEN_ADDRESS");
        if clean {
            command.arg("--clean");
        }
        if let Some(home) = isolated_home {
            std::fs::create_dir_all(home)?;
            let runtime = home.join("runtime");
            std::fs::create_dir_all(&runtime)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))?;
            }
            command
                .current_dir(home)
                .env("HOME", home)
                .env("USERPROFILE", home)
                .env("XDG_CONFIG_HOME", home.join("config"))
                .env("XDG_DATA_HOME", home.join("data"))
                .env("XDG_STATE_HOME", home.join("state"))
                .env("XDG_CACHE_HOME", home.join("cache"))
                .env("XDG_RUNTIME_DIR", runtime)
                .env("NVIM_LOG_FILE", home.join("nvim.log"))
                .env("LOCALAPPDATA", home.join("data"))
                .env("APPDATA", home.join("config"));
            command
                .env("NVIM_APPNAME", "nvim")
                .env_remove("VIMINIT")
                .env_remove("EXINIT");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                error("Could not start nvim. Install Neovim 0.9+ or use :set editor builtin")
            })?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| error("Neovim stdin unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| error("Neovim stdout unavailable"))?;
        let (outgoing, commands) = mpsc::sync_channel(256);
        // A frozen plugin must not block the application's event loop on a pipe
        // write. Both directions have bounded queues and requests have deadlines.
        std::thread::spawn(move || {
            for value in commands {
                if rmpv::encode::write_value(&mut input, &value).is_err() || input.flush().is_err()
                {
                    break;
                }
            }
        });
        let (tx, frames) = mpsc::sync_channel(256);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            while let Ok(value) = rmpv::decode::read_value_with_max_depth(&mut reader, 64) {
                if tx.send(value).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            child,
            input: outgoing,
            frames,
            queued: VecDeque::new(),
            next: 0,
        })
    }
    fn send(&mut self, value: Value) -> io::Result<()> {
        self.input.try_send(value).map_err(|_| {
            error("Neovim input queue is full or closed; use F2 to recover the editor")
        })
    }
    pub fn notify(&mut self, method: &str, params: Vec<Value>) -> io::Result<()> {
        self.send(Value::Array(vec![
            2.into(),
            method.into(),
            Value::Array(params),
        ]))
    }
    pub fn reply(&mut self, id: u64, result: io::Result<Value>) -> io::Result<()> {
        let (err, result) = match result {
            Ok(value) => (Value::Nil, value),
            Err(err) => (err.to_string().into(), Value::Nil),
        };
        self.send(Value::Array(vec![1.into(), id.into(), err, result]))
    }
    pub fn call(&mut self, method: &str, params: Vec<Value>) -> io::Result<Value> {
        self.next += 1;
        let id = self.next;
        self.send(Value::Array(vec![
            0.into(),
            id.into(),
            method.into(),
            Value::Array(params),
        ]))?;
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() >= until {
                return Err(error(
                    "Neovim RPC timed out; try clean configuration or the built-in editor",
                ));
            }
            let remaining = until.saturating_duration_since(Instant::now());
            let value = self.frames.recv_timeout(remaining).map_err(|_| {
                error(
                    "Neovim stopped responding. Try :set nvim-config clean or :set editor builtin",
                )
            })?;
            if let Some(fields) = value.as_array() {
                if fields.len() == 4
                    && fields[0].as_u64() == Some(1)
                    && fields[1].as_u64() == Some(id)
                {
                    return if fields[2].is_nil() {
                        Ok(fields[3].clone())
                    } else {
                        Err(error("Neovim rejected an editor operation; inspect its messages or try clean configuration"))
                    };
                }
            }
            if let Some(message) = decode(value) {
                // A plugin can issue a write while an RPC transition is in flight.
                // Reject it explicitly, rather than deadlocking or bypassing host saving.
                if let Message::Request(id, _, _) = message {
                    self.reply(id, Err(error("Editor transition in progress; retry :write after returning to the note")))?;
                } else {
                    self.queued.push_back(message);
                }
            }
        }
    }
    pub fn poll(&mut self) -> Vec<Message> {
        let mut messages: Vec<_> = self.queued.drain(..).collect();
        for _ in 0..512 {
            match self.frames.try_recv() {
                Ok(value) => {
                    if let Some(message) = decode(value) {
                        messages.push(message);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    messages.push(Message::Closed);
                    break;
                }
            }
        }
        messages
    }
}

fn decode(value: Value) -> Option<Message> {
    let fields = value.as_array()?;
    match fields.first()?.as_u64()? {
        2 if fields.len() == 3 => Some(Message::Notification(
            fields[1].as_str()?.into(),
            fields[2].as_array()?.clone(),
        )),
        0 if fields.len() == 4 => Some(Message::Request(
            fields[1].as_u64()?,
            fields[2].as_str()?.into(),
            fields[3].as_array()?.clone(),
        )),
        _ => None,
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        // The host has already saved or retained its independent text snapshot.
        // Reap only our child; never kill other Neovim instances or plugin jobs.
        if self.child.try_wait().is_ok_and(|status| status.is_some()) {
            return;
        }
        // Let Neovim clean up its own jobs/runtime files. Never wait indefinitely
        // for plugin exit hooks or a broken RPC stream.
        let _ = self.notify("nvim_command", vec!["qa!".into()]);
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            if self.child.try_wait().is_ok_and(|status| status.is_some()) {
                return;
            }
            for message in self.poll() {
                if let Message::Request(id, _, _) = message {
                    let _ = self.reply(id, Err(error("Paper Trail is closing this editor; text is saved or retained by the host")));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_executable_is_actionable_without_starting_a_shell() {
        let root = tempfile::tempdir().unwrap();
        let result = Rpc::spawn_command(Command::new(root.path().join("missing-nvim")), true, None);
        let message = result.err().unwrap().to_string();
        assert!(message.contains("Install Neovim"));
        assert!(message.contains("builtin"));
    }
}
