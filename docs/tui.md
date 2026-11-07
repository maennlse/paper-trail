# Paper Trail TUI

`paper-trail-tui` is a Ratatui terminal frontend sharing Paper Trail's portable
core and filesystem storage. It needs no GTK, Python, Rust installation, network
service, or third-party runtime libraries on the destination machine.

## Run

Download the archive for your OS and CPU from GitHub Releases, or the
**Standalone TUI** workflow's artifacts for development builds. Check its SHA-256
checksum and extract it. Linux/macOS archives retain
the executable permission. Run in an interactive terminal:

```sh
./paper-trail-tui
./paper-trail-tui --notes-dir /path/to/existing/notes
./paper-trail-tui --config /path/to/paper-trail/settings.json
```

On Windows, use `paper-trail-tui.exe` in Windows Terminal or another terminal with
ANSI/VT support. Paths passed as CLI arguments need normal shell quoting when
they contain spaces. `--help` and `--version` work without an interactive terminal.

The first run creates only the default notes directory. Explicit folders must
already exist. Without `--config`, the TUI prefers existing native desktop
settings, then existing Linux Flatpak settings, then native defaults. Native
config locations follow the platform (XDG on Linux, Application Support on macOS,
and the roaming application-data directory on Windows). If both native and
Flatpak settings exist, use `--config` to select Flatpak explicitly.

## Keys and editing

Three panes show folders, notes, and the editor. **H / L** switches panes left/right;
**Tab / Shift-Tab** also works. In insert mode, H and L remain typed characters.
Use **j/k** or arrows to browse: the highlighted note is immediately previewed
without moving keyboard focus. **Enter** focuses the editor; the note is already
loaded. Pending edits are saved before a different note is shown, and a failed
save leaves the original note, selection, and edits intact.

The folder column defaults to **16 terminal cells**, configurable with
`:set folder-width 10` through `:set folder-width 30`. Long names use an ellipsis;
`:path` opens a wrapping, scrollable full-path dialog. Below 80 columns the folder
pane hides automatically; pane navigation skips it. Folders remain accessible
through `:folder PATH` or `:folder NUMBER`.

A **›** marks the highlighted list item, **●** marks the displayed note/active
folder, and a cyan border identifies keyboard focus. A contextual footer shows
navigation/editing shortcuts and editor mode.

**Mouse support** is enabled by default: single-click notes to preview, click a
folder to open it while keeping focus in Folders, click panes to focus, and click editor text to position the
cursor. The wheel browses lists or scrolls the editor/help under the pointer.
Editor hit testing accounts for scroll position, line numbers, tabs, and wide
characters. Start with `--no-mouse` to retain native terminal mouse selection;
terminals that support it also permit Shift-drag to bypass application capture.
Mouse capture is released before external editing and when the TUI exits.

**?** shows scrollable help. List **/** searches filenames, titles, and full note bodies
across saved folders; **Esc** clears the filter. **p** toggles a selected note's pin.

Editing uses a practical Vim-style subset, **not full Vim compatibility**:

| Keys                            | Action                                       |
| ------------------------------- | -------------------------------------------- |
| `i`, `a`, `I`, `A`              | Insert / append at cursor or line boundary   |
| `o`, `O`                        | Insert a line below / above                  |
| `Esc`                           | Return to normal mode                        |
| `h j k l`, `w b`, `0 $`, `gg G` | Cursor movement                              |
| `v`, then movement              | Visual selection                             |
| `y`, `d` / `x` in visual mode   | Copy / cut selection                         |
| `x`, `dd`, `yy`, `p`            | Delete character/line, copy line text, paste |
| `u`, `Ctrl-R`                   | Undo / redo                                  |
| `/`, `n`, `N` in editor         | Literal find, next / previous match          |
| `Ctrl-N`, `Ctrl-S`, `Ctrl-Q`    | New note, save, save and quit                |

Counts, macros, registers, Vim plugins, and system-clipboard integration are not
implemented. Copy/paste uses an internal editor register; terminal bracketed
paste is supported in insert mode. Long lines scroll horizontally rather than
soft-wrap. Syntax highlighting and desktop-only features such as printing are
not part of this frontend.

## Commands

Press **:** in normal mode, type a command, then **Enter**. **Esc** cancels.

**Tab / Shift-Tab** completes and cycles matching commands, `:set` options and
folder widths, preset colors, and saved folder numbers. `:folder` and `:move`
complete directory paths; `:saveas` completes files and directories. Paths with
spaces or Unicode and `~/` home paths are supported. A unique directory can be
completed further with another Tab. Suggestions appear above the prompt and
never execute a command; Enter is still required. Typing, pasting, or Backspace
starts a new completion cycle. Tab switches panes only when no prompt is open.

| Command                               | Action                                              |
| ------------------------------------- | --------------------------------------------------- |
| `:w`, `:q`, `:wq`                     | Save / save and quit; failed saves prevent quitting |
| `:q!`                                 | Explicitly discard pending edits and quit           |
| `:new [NAME]`, `:rename NAME`         | Create / rename a note                              |
| `:move PATH` or `:move NUMBER`        | Move into an existing folder                        |
| `:folder PATH` or `:folder NUMBER`    | Add / select an existing folder                     |
| `:all`, `:search TEXT`                | Show all notes / search across folders              |
| `:close-folder`                       | Remove active bookmark, never its files             |
| `:folder-up`, `:folder-down`          | Reorder the highlighted folder                      |
| `:pin`, `:color #rrggbb`              | Toggle current note pin / set active folder color   |
| `:color preset-0` through `preset-11` | Use a built-in folder color                         |
| `:delete`                             | Delete current note after typing `yes` and Enter    |
| `:replace 'OLD' 'NEW'`                | Undoable replacement of all literal matches         |
| `:edit`                               | Save and launch an external editor                  |
| `:refresh`, `:reload`                 | Rescan notes / reload a clean current note          |
| `:saveas PATH`                        | Save pending text into a new file, never overwrite  |
| `:reload!`                            | Explicitly discard edits and load the disk version  |
| `:set number`, `:set nonumber`        | TUI-only line-number preference                     |
| `:set folder-width N`                 | TUI-only folder width, 10–30 terminal cells         |
| `:path`                               | Show the full highlighted folder/current note path  |

Paths after `:folder`, `:move`, `:rename`, and `:saveas` can contain spaces without
quotes. `:replace` arguments use shell-style quotes. Moves and renames resolve
filename collisions without overwriting another note. Deletion permanently
removes the file, rather than using a desktop trash service.

**External editing:** `$VISUAL` takes precedence over `$EDITOR`; default `vi` on
Unix or `notepad` on Windows. Editor arguments are parsed without running a shell;
the note path is passed as a separate argument. Use forward slashes in quoted
Windows editor paths, for example `"C:/Program Files/Editor/editor.exe" --wait`.
Configure graphical editors to wait for their document to close. The terminal
is restored before launching the editor and reinitialized afterwards. A successful
editor exit reloads the note; after a failed exit, use `:reload` explicitly.
Only external editing needs another program installed.

## Shared data and safety

Notes, bookmarks, pins, folder colors, and language overrides share the desktop
format. TUI preferences live in `tui.json` alongside the selected settings file;
desktop window/layout preferences are not changed by TUI commands. Settings
updates preserve unknown fields, merge unrelated concurrent changes, and reject
conflicting changes. After a settings conflict, restart to reload preferences;
save any pending text first.

Both frontends check note contents before autosaving. If another client changed
the file, the save is blocked, pending text remains in memory, and TUI autosave
pauses. Use `:saveas PATH` for a separate copy or `:reload!` to discard your edits
explicitly. Cooperating Paper Trail writers use advisory lockfiles beneath
`.papertrail-locks/`; those directories are not notes. Unrelated editors do not
honor these locks: content checks detect existing changes, but cannot prevent an
external write racing the final filesystem operation. Avoid editing the same
note simultaneously in unrelated programs.

Autosave runs after 350 ms of inactivity. Quit normally before closing a terminal.
On a terminal error or Unix termination signal, terminal state is restored and
pending text is saved if possible. If a save fails, the TUI attempts a separate
recovery file under the platform's local data directory, `paper-trail/recovery/`,
and prints its path. Forced termination (`SIGKILL`, a power failure, or killing a
Windows process) cannot guarantee recovery of edits not yet autosaved.

## Build and distribution

```sh
cargo run --locked --bin paper-trail-tui
cargo test --locked -p papertrail-core -p papertrail-storage -p papertrail-tui
cargo build --release --locked --bin paper-trail-tui
```

Building the TUI requires Rust 1.88 or newer. Its dependency graph excludes
GTK/GLib. The six-target CI matrix builds,
tests, verifies linkage, and packages:

| OS      | Architectures | Linking                                                                     |
| ------- | ------------- | --------------------------------------------------------------------------- |
| Linux   | x86_64, ARM64 | Fully static musl; no ELF interpreter or shared-library dependencies        |
| Windows | x86_64, ARM64 | Static Rust dependencies and C runtime; built-in Windows system DLLs remain |
| macOS   | x86_64, ARM64 | Static Rust dependencies; Apple system libraries remain dynamically linked  |

macOS does not support fully static executables. Artifacts are not signed or
notarized; OS download-security prompts can apply. Build on an older supported
macOS SDK/deployment target if compatibility with older macOS versions is needed.

For a static Linux build, install the musl compiler and Rust target, then run:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl --bin paper-trail-tui
python3 build-aux/package_tui.py x86_64-unknown-linux-musl
```

Windows MSVC builds use `RUSTFLAGS="-C target-feature=+crt-static"` with an explicit
`--target`. `build-aux/package_tui.py` rejects dynamically linked Linux builds,
Windows artifacts importing the dynamic MSVC/UCRT runtime, and non-system macOS
dependencies. Python is build tooling only; it is not included in the executable.
