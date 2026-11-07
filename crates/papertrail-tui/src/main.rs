// SPDX-License-Identifier: GPL-3.0-or-later
mod app;
mod completion;
mod config;
mod editor;
#[cfg(test)]
mod tests;
mod ui;

use app::App;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, IsTerminal},
    process::Command,
    time::Duration,
};

struct TerminalGuard;
impl TerminalGuard {
    fn enter(mouse: bool) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        if mouse {
            execute!(io::stdout(), EnableMouseCapture)?;
        }
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}
fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

fn run(app: &mut App) -> io::Result<()> {
    let interrupted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGINT,
    ] {
        signal_hook::flag::register(signal, interrupted.clone())?;
    }
    let mut guard = TerminalGuard::enter(app.mouse_enabled)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    while !app.quit {
        if interrupted.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::Error::other("Termination requested"));
        }
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(Duration::from_millis(100))? {
            let result = match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                Event::Mouse(mouse) => app.mouse(mouse),
                Event::Paste(text) => {
                    app.paste(&text);
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                app.status = error.to_string();
            }
        }
        if let Err(error) = app.tick() {
            app.status = error.to_string();
        }
        if app.external {
            app.external = false;
            let path = app.current_path()?;
            let parts = match config::editor_command() {
                Ok(parts) => parts,
                Err(error) => {
                    app.status = error.to_string();
                    continue;
                }
            };
            drop(guard);
            let result = Command::new(&parts[0])
                .args(&parts[1..])
                .arg(&path)
                .status();
            guard = TerminalGuard::enter(app.mouse_enabled)?;
            terminal.clear()?;
            match result {
                Ok(status) if status.success() => {
                    if let Err(error) = app.reload(false).and_then(|()| app.refresh()) {
                        app.status = error.to_string();
                    }
                }
                Ok(status) => {
                    app.status = format!("Editor exited with {status}; :reload to read its changes")
                }
                Err(error) => app.status = format!("Could not launch editor: {error}"),
            }
        }
    }
    drop(terminal);
    drop(guard);
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", config::HELP);
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("paper-trail-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "An interactive terminal is required. Use --help for options.",
        ));
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    let mut app = App::new(config::Paths::discover(args)?)?;
    let result = run(&mut app);
    if result.is_err() && app.session.is_dirty() {
        eprintln!("Terminal error; trying to save pending edits...");
        if let Err(error) = app.flush() {
            eprintln!("Could not save edits: {error}");
            match save_recovery(&app.session.text) {
                Ok(path) => eprintln!("Pending text recovered to {}", path.display()),
                Err(error) => eprintln!("Could not write recovery copy: {error}"),
            }
        }
    }
    if result.is_err() {
        if let Err(error) = app.persist() {
            eprintln!("Could not persist settings: {error}");
        }
    }
    result
}

fn save_recovery(text: &str) -> io::Result<std::path::PathBuf> {
    use std::io::Write;
    let root = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("paper-trail/recovery");
    std::fs::create_dir_all(&root)?;
    for index in 0..1000 {
        let path = root.join(format!("tui-{}-{index}.txt", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("No unused recovery filename"))
}
