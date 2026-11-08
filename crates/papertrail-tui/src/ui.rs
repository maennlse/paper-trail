// SPDX-License-Identifier: GPL-3.0-or-later
use crate::app::{App, Focus, PromptKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const KEYS: &str = r#"PAPER TRAIL — TERMINAL HELP

H/L or Tab/Shift-Tab   Switch panes (H/L type normally in insert mode)
j/k or arrows         Browse; highlighted notes preview automatically
Enter                 Open folder / focus editor
Click                 Select folder/note or focus a pane; position cursor
Mouse wheel           Browse lists / scroll editor or help
--no-mouse            Disable capture for native terminal text selection
/                     Search all notes (lists), or find in editor
p                     Toggle highlighted note's pin
Ctrl-N / Ctrl-S       New note / save
Ctrl-Q                Save, persist settings, and quit
Esc                   Normal mode / cancel prompt / clear list search
› marks selection; ● marks the displayed note; cyan border marks focus.

VIM SUBSET (not a full Vim emulator)
i a I A o O           Insert / append / new line
h j k l w b 0 $       Move; gg / G jump top / bottom
v                     Visual selection; y copies, d/x cuts
x dd yy p             Delete character/line, copy line, paste
u / Ctrl-R            Undo / redo (including replacement)
/ text, n / N         Literal find, next / previous match

COMMANDS (: opens the prompt; Enter executes)
:w  :q  :wq           Save / save-and-quit; :q! explicitly discards
:new [NAME]           Create note; :rename NAME; :move PATH or NUMBER
:folder PATH          Add/select existing directory; :all shows all folders
:close-folder         Remove active bookmark, not files
:folder-up/down       Reorder highlighted folder
:color #rrggbb        Or preset-0 through preset-11; :pin toggles pin
:delete               Type yes and Enter to confirm
:search TEXT          Search filename/title/full content across folders
:replace 'OLD' 'NEW'   Replace all literal matches (undoable)
:edit                 Save, suspend TUI, launch $VISUAL / $EDITOR
:refresh / :reload    Rescan notes / reload current note
:saveas PATH          Save edits to a new file (never overwrites)
:reload!              Explicitly discard edits and load disk version
:set number           Or :set nonumber (TUI-only preference)
:set folder-width 16  Folder column width, 10–30; hidden below 80 columns
:path                 Show the full highlighted folder/current note path
:sync-configure URL ISSUER CLIENT ['SCOPES']  Sync the active folder
:login               Device code sign-in (works over SSH)
:login-show          Show the full verification URL/code
:login-cancel        Cancel pending device sign-in (or Ctrl-C)
:login-browser       Optional browser/PKCE login on this machine
:unlock :sync        Separate vault unlock and manual sync
:create-vault         Create vault; save and confirm recovery key
:recover-vault        Unlock using recovery key
:change-passphrase    Rewrap vault; save replacement recovery key
:lock :logout         Clear vault key / credentials; files stay plaintext
:sync-status          Show synchronization status
:sync-folders         List folder IDs, selection and local mappings
:sync-add-folder PATH / :sync-remove-folder PATH  Select/unselect; never delete
:sync-map-folder ID PATH / :sync-download-dir PATH  Device-local directories
:sync-map-legacy NOTE_ID FOLDER_ID  Confirm an ambiguous legacy assignment
:sync-accept-remote ID Accept remote name/color; keep local directory
Tab / Shift-Tab       Complete commands/options/paths while : prompt is open

Paths after :folder, :move, :rename, :saveas may contain spaces
without quotes; :replace uses shell-style quoted arguments.
Autosave runs 350 ms after editing. Failed saves prevent preview switches.
On conflict, edits remain in memory and autosave pauses; :saveas preserves
a separate copy. Quit normally before closing your terminal.
External changes are checked on save; :reload refreshes a clean note.

Arrows / PgUp / PgDn / mouse wheel scroll help. Esc closes."#;

fn block(title: String, active: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if active { Color::Cyan } else { Color::DarkGray }))
}

pub fn panes(area: Rect, folder_width: u16) -> [Rect; 3] {
    let folders = if area.width >= 80 {
        folder_width.clamp(10, 30)
    } else {
        0
    };
    let columns = Layout::horizontal([
        Constraint::Length(folders),
        Constraint::Length((area.width / 4).clamp(20, 36)),
        Constraint::Min(20),
    ])
    .split(area);
    [columns[0], columns[1], columns[2]]
}

fn ellipsize(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let size = c.width().unwrap_or(0);
        if used + size >= width {
            break;
        }
        out.push(c);
        used += size;
    }
    out.push('…');
    out
}

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 55 || area.height < 10 {
        app.panes = [Rect::default(); 3];
        frame.render_widget(
            Paragraph::new(
                "Please enlarge the terminal to at least 55 × 10. Ctrl-Q saves and quits.",
            ),
            area,
        );
        return;
    }
    let login_height = if app.device_prompt.is_some() { 5 } else { 0 };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2 + login_height)])
        .split(area);
    let columns = panes(vertical[0], app.preferences.folder_width);
    app.panes = columns;
    app.folders_visible = columns[0].width > 0;
    if !app.folders_visible && app.focus == Focus::Folders {
        app.focus = Focus::Notes;
    }
    let folders: Vec<ListItem> = app
        .settings
        .note_folders
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let active = *path == app.settings.notes_dir;
            let unavailable = !path.is_dir();
            let name = app.settings.folder_name(path);
            let token = app
                .settings
                .folder_colors
                .get(path)
                .map(String::as_str)
                .unwrap_or("preset-0");
            ListItem::new(ellipsize(
                &format!(
                    "{} {}{}{}",
                    index + 1,
                    if unavailable {
                        "! "
                    } else if active {
                        "● "
                    } else {
                        "  "
                    },
                    name,
                    if unavailable { " [!]" } else { "" }
                ),
                usize::from(columns[0].width.saturating_sub(2)),
            ))
            .style(Style::default().fg(if unavailable {
                Color::DarkGray
            } else {
                folder_color(token)
            }))
        })
        .collect();
    app.folder_state.select(Some(app.folder_index));
    frame.render_stateful_widget(
        List::new(folders)
            .block(block("Folders".into(), app.focus == Focus::Folders))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        columns[0],
        &mut app.folder_state,
    );
    let notes: Vec<ListItem> = app
        .notes
        .iter()
        .map(|note| {
            let pin = if app.settings.pinned_notes.contains(&note.path) {
                "* "
            } else {
                "  "
            };
            let opened = if app
                .session
                .current
                .as_ref()
                .is_some_and(|n| n.path == note.path)
            {
                "● "
            } else {
                "  "
            };
            let folder = note
                .path
                .parent()
                .map(|p| app.settings.folder_name(p))
                .unwrap_or_default();
            ListItem::new(vec![
                Line::from(format!("{opened}{pin}{}", note.title)),
                Line::styled(
                    format!(
                        "  {folder}/{}",
                        note.path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        })
        .collect();
    app.note_state.select(if notes.is_empty() {
        None
    } else {
        Some(app.note_index)
    });
    let title = format!(
        "{} notes{}",
        app.notes.len(),
        if !app.query.is_empty() {
            format!(" /{}", app.query)
        } else if app.all {
            " · All".into()
        } else {
            String::new()
        }
    );
    frame.render_stateful_widget(
        List::new(notes)
            .block(block(title, app.focus == Focus::Notes))
            .highlight_symbol("› ")
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        columns[1],
        &mut app.note_state,
    );
    let title = app
        .session
        .current
        .as_ref()
        .map(|n| {
            n.path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| ":new to create a note".into());
    let dirty = if app.session.is_dirty() { " [+]" } else { "" };
    app.editor.area.set_block(block(
        format!("{title}{dirty} · {:?}", app.editor.mode),
        app.focus == Focus::Editor,
    ));
    app.editor
        .area
        .set_cursor_line_style(Style::default().add_modifier(Modifier::UNDERLINED));
    app.editor
        .area
        .set_cursor_style(if app.focus == Focus::Editor {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        });
    app.editor
        .area
        .set_line_number_style(if app.preferences.hide_numbers {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        });
    if app.preferences.hide_numbers {
        app.editor.area.remove_line_number();
    }
    app.editor.track_view(
        columns[2].width.saturating_sub(2),
        columns[2].height.saturating_sub(2),
    );
    frame.render_widget(&app.editor.area, columns[2]);
    let status = if let Some(prompt) = &app.prompt {
        let prefix = match prompt.kind {
            PromptKind::Command => ":",
            PromptKind::NotesSearch | PromptKind::Find => "/",
            PromptKind::Delete => "Delete note? Type yes: ",
            PromptKind::Unlock => "Vault passphrase: ",
            PromptKind::CreateVault => "New strong vault passphrase (16+ characters): ",
            PromptKind::RecoverVault => "Recovery key: ",
            PromptKind::ChangePassphrase => "New strong vault passphrase (16+ characters): ",
            PromptKind::ConfirmRecovery => "Saved the recovery key? Type yes: ",
        };
        let text = if prompt.kind.secret() {
            "•".repeat(prompt.text.chars().count())
        } else {
            prompt.text.clone()
        };
        format!("{prefix}{text}")
    } else {
        app.status.clone()
    };
    let footer = Layout::vertical([
        Constraint::Length(login_height),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(vertical[1]);
    if let Some(prompt) = &app.device_prompt {
        let uri = if prompt.verification_uri.width() > area.width.saturating_sub(4) as usize {
            "Use :login-show for the full verification URL"
        } else {
            &prompt.verification_uri
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{uri}\nCode: {} · expires in {}s\nKeep editing · Ctrl-C or :login-cancel cancels",
                prompt.user_code.as_str(),
                prompt.seconds_left()
            ))
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title("Sign in on another device · :login-show")),
            footer[0],
        );
    }
    frame.render_widget(Paragraph::new(status), footer[1]);
    let keys = match app.focus {
        Focus::Folders => "FOLDERS · j/k · Enter open · :path",
        Focus::Notes => "NOTES · j/k preview · Enter edit",
        Focus::Editor if app.editor.mode == crate::editor::Mode::Insert => {
            "INSERT · Esc normal · Ctrl-S save"
        }
        Focus::Editor if app.editor.mode == crate::editor::Mode::Visual => {
            "VISUAL · y copy · d cut · Esc normal"
        }
        Focus::Editor => "NORMAL · i insert · / find · : cmd",
    };
    let navigation = if app.focus == Focus::Editor && app.editor.mode == crate::editor::Mode::Insert
    {
        "Tab panes"
    } else {
        "H/L/Tab · ? help"
    };
    let hints = if let Some(prompt) = &app.prompt {
        if prompt.kind == PromptKind::Command {
            "COMMAND · Tab/Shift-Tab complete · Enter run · Esc cancel".into()
        } else {
            "PROMPT · Enter confirm · Esc cancel".into()
        }
    } else {
        format!("{keys} · {navigation}")
    };
    frame.render_widget(
        Paragraph::new(hints).style(Style::default().fg(Color::DarkGray)),
        footer[2],
    );
    if let Some(prompt) = app.prompt.as_ref().filter(|p| p.completion.active) {
        let completion = &prompt.completion;
        let rows = completion.items.len().clamp(1, 5) as u16;
        let height = (rows + 2).min(vertical[0].height);
        let popup = Rect::new(
            area.x,
            vertical[0].bottom().saturating_sub(height),
            area.width,
            height,
        );
        frame.render_widget(Clear, popup);
        let border = Block::bordered().title(format!(
            "Complete · Tab/Shift-Tab cycle · Enter runs · {} matches",
            completion.items.len()
        ));
        if completion.items.is_empty() {
            frame.render_widget(
                Paragraph::new(
                    completion
                        .message
                        .as_deref()
                        .unwrap_or("No matches · edit the prefix or Esc to cancel"),
                )
                .block(border),
                popup,
            );
        } else {
            let items: Vec<_> = completion
                .items
                .iter()
                .map(|s| ListItem::new(format!(":{s}")))
                .collect();
            let mut state = ListState::default().with_selected(completion.selected);
            frame.render_stateful_widget(
                List::new(items)
                    .block(border)
                    .highlight_symbol("› ")
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
                popup,
                &mut state,
            );
        }
    }
    if let Some(key) = &app.recovery_key {
        let popup = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            vertical[0].height.saturating_sub(2),
        );
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new(format!("Save this recovery key somewhere safe:\n\n{}\n\nIt is shown only now. Losing your passphrase, recovery key, and all unlocked devices means your notes cannot be recovered.\n\nType yes in the prompt below only after saving it. Esc cancels. Local files remain plaintext.",key.as_str())).wrap(Wrap { trim:false }).block(Block::bordered().title("Save Recovery Key")),popup);
    }
    if app.help {
        let popup = ratatui::layout::Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(app.path_details.as_deref().unwrap_or(KEYS))
                .scroll((app.help_scroll, 0))
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(if app.path_details.is_some() {
                    "Full path · ↑↓/PgUp/PgDn scroll · Esc closes"
                } else {
                    "Help · ↑↓/PgUp/PgDn scroll · Esc closes"
                })),
            popup,
        );
    }
    if app.device_details {
        if let Some(prompt) = &app.device_prompt {
            let popup = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            frame.render_widget(Clear, popup);
            frame.render_widget(Paragraph::new(format!("Open this HTTPS URL in a browser on any trusted device:\n\n{}\n\nEnter this user code:\n\n{}\n\nExpires in {} seconds. Approve only the sign-in you started.\n\nNo browser or forwarded port is needed on this SSH host. Vault unlock is a separate step. The SSH host must be trusted with your notes.\n\nEsc / Enter closes this view; Ctrl-C cancels sign-in.",prompt.verification_uri,prompt.user_code.as_str(),prompt.seconds_left())).wrap(Wrap {trim:false}).scroll((app.help_scroll,0)).block(Block::bordered().title("Device Sign-In · ↑↓/PgUp/PgDn scroll")),popup);
        }
    }
}

fn folder_color(token: &str) -> Color {
    if let Some(rgb) = token
        .strip_prefix('#')
        .and_then(|s| u32::from_str_radix(s, 16).ok())
    {
        return Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    }
    const COLORS: [Color; 12] = [
        Color::Blue,
        Color::Cyan,
        Color::Green,
        Color::Yellow,
        Color::Red,
        Color::Magenta,
        Color::LightBlue,
        Color::LightCyan,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightRed,
        Color::LightMagenta,
    ];
    COLORS[token
        .strip_prefix("preset-")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        % COLORS.len()]
}
