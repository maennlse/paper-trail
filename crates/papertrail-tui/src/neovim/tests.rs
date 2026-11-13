use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn available() -> bool {
    let found = std::process::Command::new("nvim")
        .arg("--version")
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|output| {
            let version = String::from_utf8_lossy(&output.stdout);
            let mut parts = version
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches("NVIM v")
                .split('.');
            let major = parts.next().and_then(|value| value.parse::<u32>().ok());
            let minor = parts.next().and_then(|value| value.parse::<u32>().ok());
            output.status.success()
                && matches!((major, minor), (Some(major), Some(minor)) if major > 0 || minor >= 9)
        });
    if !found {
        assert!(
            std::env::var_os("PAPERTRAIL_NVIM_TESTS").is_none(),
            "Neovim integration tests require nvim 0.9+ on PATH"
        );
        eprintln!("Skipping optional Neovim integration test: nvim 0.9+ is not installed");
    }
    found
}

#[test]
fn real_neovim_edits_undoes_redoes_and_pastes_without_writing_directly() {
    if !available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("a note ' with | characters.txt");
    let original = "one two\nsecond 界";
    std::fs::write(&path, original).unwrap();
    let mut editor =
        Neovim::start_isolated(true, Some(&path), original, &root.path().join("home")).unwrap();
    let options = editor
        .rpc
        .call(
            "nvim_exec_lua",
            vec![
                "local b=...; return {vim.bo[b].buftype,vim.bo[b].swapfile}".into(),
                Value::Array(vec![editor.buffer.into()]),
            ],
        )
        .unwrap();
    assert_eq!(options.as_array().unwrap()[0].as_str(), Some("acwrite"));
    assert_eq!(options.as_array().unwrap()[1].as_bool(), Some(false));
    editor.input("gg0dw").unwrap();
    wait_for_text(&mut editor, "two\nsecond 界");
    editor.input("u").unwrap();
    wait_for_text(&mut editor, original);
    editor.input("<C-r>").unwrap();
    wait_for_text(&mut editor, "two\nsecond 界");
    editor.input("A").unwrap();
    editor.paste("<Esc>:wq\nliteral λ").unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let update = editor.poll();
        if update
            .events
            .iter()
            .any(|event| matches!(event, Event::Text(text) if text.contains("<Esc>:wq\nliteral λ")))
        {
            break;
        }
        assert!(!update.closed);
        assert!(
            std::time::Instant::now() < until,
            "Literal paste did not reach Neovim"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

fn wait_for_text(editor: &mut Neovim, expected: &str) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let update = editor.poll();
        if update
            .events
            .iter()
            .any(|event| matches!(event, Event::Text(text) if text == expected))
        {
            return;
        }
        assert!(!update.closed);
        assert!(
            std::time::Instant::now() < until,
            "Expected Neovim text update"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn user_configuration_is_loaded_only_when_clean_mode_is_disabled() {
    if !available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let init = root.path().join("config/nvim/init.lua");
    std::fs::create_dir_all(init.parent().unwrap()).unwrap();
    std::fs::write(init, "vim.g.papertrail_test_config = true; vim.o.tabstop=3").unwrap();
    let path = root.path().join("note.txt");
    let mut editor = Neovim::start_isolated(false, Some(&path), "text", root.path()).unwrap();
    let value = editor
        .rpc
        .call(
            "nvim_exec_lua",
            vec![
                "return {vim.g.papertrail_test_config,vim.o.tabstop}".into(),
                Value::Array(vec![]),
            ],
        )
        .unwrap();
    assert_eq!(value.as_array().unwrap()[0].as_bool(), Some(true));
    assert_eq!(value.as_array().unwrap()[1].as_u64(), Some(3));
    drop(editor);
    let mut editor = Neovim::start_isolated(true, Some(&path), "text", root.path()).unwrap();
    let value = editor
        .rpc
        .call(
            "nvim_exec_lua",
            vec![
                "return vim.g.papertrail_test_config == nil".into(),
                Value::Array(vec![]),
            ],
        )
        .unwrap();
    assert_eq!(value.as_bool(), Some(true));
}

#[test]
fn real_neovim_resize_mouse_and_unicode_render_inside_the_editor_rectangle() {
    if !available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.txt");
    let mut editor =
        Neovim::start_isolated(true, Some(&path), "first界\nsecond line", root.path()).unwrap();
    editor.resize(40, 12);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.poll();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 50, 20));
        buffer.set_string(0, 0, "outside", ratatui::style::Style::default());
        editor.render(Rect::new(2, 3, 40, 12), &mut buffer, false);
        if buffer.cell((2, 3)).unwrap().symbol() == "f" {
            assert_eq!(buffer.cell((7, 3)).unwrap().symbol(), "界");
            assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "o");
            break;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    editor
        .mouse(
            MouseEventKind::Down(MouseButton::Left),
            1,
            3,
            KeyModifiers::NONE,
        )
        .unwrap();
    editor
        .mouse(
            MouseEventKind::Up(MouseButton::Left),
            1,
            3,
            KeyModifiers::NONE,
        )
        .unwrap();
    let cursor = editor
        .rpc
        .call("nvim_win_get_cursor", vec![0.into()])
        .unwrap();
    assert_eq!(cursor.as_array().unwrap()[0].as_u64(), Some(2));
    assert_eq!(cursor.as_array().unwrap()[1].as_u64(), Some(3));
}

#[test]
fn manual_writes_run_user_pre_and_post_hooks_and_reject_native_filename_changes() {
    if !available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let init = root.path().join("config/nvim/init.lua");
    std::fs::create_dir_all(init.parent().unwrap()).unwrap();
    std::fs::write(init, "vim.api.nvim_create_autocmd('BufWritePre',{callback=function() local l=vim.api.nvim_buf_get_lines(0,0,1,false); vim.api.nvim_buf_set_lines(0,0,1,false,{l[1]:upper()}) end}); vim.api.nvim_create_autocmd('BufWritePost',{callback=function() vim.g.papertrail_write_post=true end})").unwrap();
    let path = root.path().join("note.txt");
    let mut editor = Neovim::start_isolated(false, Some(&path), "mixed Case", root.path()).unwrap();
    editor.input(":w<CR>").unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let update = editor.poll();
        let write = update.events.into_iter().find_map(|event| match event {
            Event::Write(write) => Some(write),
            _ => None,
        });
        if let Some(write) = write {
            assert_eq!(write.text, "MIXED CASE");
            std::fs::write(&path, &write.text).unwrap();
            editor.reply_write(write.id, Ok(())).unwrap();
            break;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let post = editor
        .rpc
        .call(
            "nvim_exec_lua",
            vec![
                "return vim.g.papertrail_write_post".into(),
                Value::Array(vec![]),
            ],
        )
        .unwrap();
    assert_eq!(post.as_bool(), Some(true));
    let renamed = root.path().join("not-adopted.txt");
    editor
        .rpc
        .call(
            "nvim_buf_set_name",
            vec![
                editor.buffer.into(),
                renamed.to_string_lossy().as_ref().into(),
            ],
        )
        .unwrap();
    editor.input(":w<CR><Esc>").unwrap();
    let message = editor
        .rpc
        .call(
            "nvim_exec_lua",
            vec!["return vim.v.errmsg".into(), Value::Array(vec![])],
        )
        .unwrap();
    assert!(message.as_str().unwrap().contains("rename/saveas"));
    assert!(!renamed.exists());
    assert!(!editor
        .poll()
        .events
        .iter()
        .any(|event| matches!(event, Event::Write(_))));
}

#[test]
fn keys_keep_neovim_commands_and_literal_less_than_distinct() {
    assert_eq!(
        key_notation(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE)).as_deref(),
        Some(":")
    );
    assert_eq!(
        key_notation(KeyEvent::new(KeyCode::Char('<'), KeyModifiers::NONE)).as_deref(),
        Some("<LT>")
    );
    assert_eq!(
        key_notation(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)).as_deref(),
        Some("<C-r>")
    );
    assert_eq!(
        key_notation(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)).as_deref(),
        Some("<S-Tab>")
    );
    assert_eq!(
        key_notation(KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE)).as_deref(),
        Some("界")
    );
}

#[test]
fn embedded_api_requires_neovim_09_or_newer() {
    for minor in [8, 9, 12] {
        let info = Value::Array(vec![
            7.into(),
            Value::Map(vec![(
                "version".into(),
                Value::Map(vec![
                    ("major".into(), 0.into()),
                    ("minor".into(), minor.into()),
                ]),
            )]),
        ]);
        assert_eq!(api_channel(&info).is_ok(), minor >= 9);
    }
    assert!(api_channel(&Value::Nil).is_err());
}

#[test]
fn linegrid_commits_complete_frames_and_keeps_rgb_highlights() {
    let mut grid = grid::Grid::default();
    let array = |values| Value::Array(values);
    grid.redraw(&[
        array(vec![
            "grid_resize".into(),
            array(vec![1.into(), 5.into(), 2.into()]),
        ]),
        array(vec![
            "hl_attr_define".into(),
            array(vec![
                7.into(),
                Value::Map(vec![("foreground".into(), 0xaabbcc.into())]),
            ]),
        ]),
        array(vec![
            "grid_line".into(),
            array(vec![
                1.into(),
                0.into(),
                0.into(),
                array(vec![
                    array(vec!["λ".into(), 7.into()]),
                    array(vec![" ".into(), 0.into(), 4.into()]),
                ]),
            ]),
        ]),
    ]);
    let area = Rect::new(0, 0, 5, 2);
    let mut buffer = Buffer::empty(area);
    grid.render(area, &mut buffer, false);
    assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), " ");
    grid.redraw(&[array(vec!["flush".into(), array(vec![])])]);
    grid.render(area, &mut buffer, false);
    assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "λ");
    assert_eq!(
        buffer.cell((0, 0)).unwrap().fg,
        ratatui::style::Color::Rgb(170, 187, 204)
    );
}
