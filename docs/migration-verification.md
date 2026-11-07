# Rust migration verification

## Automated

Core/storage tests cover metadata, Unicode casefolding, filename normalization,
sorting, settings normalization and Python fixture compatibility, pin/language
metadata remapping, fake-clock debounce, save failure recovery, note lifecycle,
collision-safe moves, an injected cross-device fallback, atomic writes,
permissions, and symlink saves (Unix).

The serial GTK workflow test loads embedded resources, checks actions, exercises
pending-save/new-note, rename/move/pins, all-folder navigation, search/replace,
undo, preferences, deletion, and window creation. It can save a PNG with
`PAPERTRAIL_SCREENSHOT`. These are behavior/smoke tests, not an automatic claim
of pixel-perfect parity.

Current-line highlighting tests toggle the actual preferences switch in light
and dark modes and compare rendered editor pixels. They verify that only one
horizontal row changes, that the highlight follows the cursor, and that no
right-margin guide/shading is enabled. Legacy `show_ruler` settings migrate to
`highlight_current_line`, preserving the saved on/off choice.
Shortcuts tests check the native close button. The headless Xvfb/xdotool runner
also sends real pointer clicks to the sidebar search and close buttons and
checks Escape while the underlying search is open. Native `GtkWindowHandle`
widgets provide header dragging without stealing button presses.
The runner uses GTK's simple input context so the isolated X server does not
connect to the host IBus service.

Editor inset tests check the native 8 px left margin with line numbers both on
and off. File type tests cover opening common extensions, uppercase extensions,
unknown/extensionless filenames, immediate re-detection after renaming, and
preserving/clearing explicit language overrides. Native filename matches retain
priority (for example, `.C` remains C++ rather than being forced to C).

Startup recovery tests cover missing/revoked portal bookmarks, permission-denied
errors, saved paths that are files, first-run default-directory creation, and
unavailable defaults. The GTK workflow also verifies the recovery alert,
creating notes in the fallback folder, searching with unavailable bookmarks,
protecting the last accessible folder, and reselecting a restored folder without
losing its colors, pins, language overrides, or existing notes.

The rebuilt Flatpak was smoke-tested with isolated config/data and a stale
document-portal path: the window opened, the sandbox default notes folder was
created, and a clean exit persisted the fallback while retaining the original
bookmark and color. No startup error or dialog-sizing warning was emitted.
On hosts where the builder's temporary rofiles mount disappears, retry the
build with `flatpak-builder --disable-rofiles-fuse`; application permissions
remain unchanged.

## Manual integration checks

Locally verified: the rendered empty-folder view was compared against the
Python Adwaita/light reference at 1320×860 on the same desktop and had **zero
different pixels**. Light editor, all-folder recent notes, dark appearance,
and preferences captures were also inspected. This is not a claim that every
state or platform has pixel-perfect parity.

The user accepted the Rust application and Flatpak, and the legacy Python app
has been removed. Keep checking these native integrations and visual states
when changing the UI. Custom editor themes are intentionally excluded.

- Empty folder, recent grid, selected note, long titles/filenames.
- Folder badges, colors, active/hover states, list scrolling/add-folder placement.
- Light/dark/system mode, custom fonts, 70–250% zoom, wrapping/current-line highlighting/line numbers.
- Note menus, rename interaction, deletion confirmation/cancel, language chooser.
- Search focus, Escape priority, Return/Shift+Return, shortcuts dialog.
- Folder ordering and titlebar dragging under native Wayland.
- File chooser portal and persistent external-folder access inside Flatpak.
- Open-containing-folder and printing, including multiple pages.
- Installed shell icon/application identity and launch outside the checkout.
- Save failure dialog and retained edits on read-only/full/unavailable storage.

## Intentional changes

- Rust instead of Python; no Python runtime or loose app assets needed.
- Adwaita light/dark/system only; no custom scheme chooser or shell recoloring.
- Correct pending-save/new-note lifecycle and folder-rename pinned paths.
- Correct recent-note activation across folders.
- Failed writes block unsafe transitions and retain dirty text.
- Writes replace files atomically (preserving symlink targets and permissions);
  other hard links to a replaced note retain their previous inode contents.
- Native GTK file dialog instead of the deprecated file-chooser API.
- An 8 px left editor text inset; line-number gutters remain native.
- Unrecognized uppercase file extensions retry native detection in lowercase.
- “Highlight Current Line” replaces the misleading “Show Ruler” preference:
  it controls horizontal cursor-row highlighting, not a vertical guide or shaded margin.

Legacy settings fixtures remain as regression coverage after removal of the
Python implementation. Windows/macOS builds of the portable crates do not imply
desktop UI support on those platforms.
