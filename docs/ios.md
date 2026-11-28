# Native iOS client

Paper Trail for iOS is an iOS 17+ SwiftUI app backed by the same Rust note,
storage, metadata, crypto, and sync crates as the desktop and TUI clients.
New notes are UTF-8 `.txt` files; existing notes may use any extension or none.
Sync encryption does **not** encrypt
local note files.

## Get an unsigned IPA without a Mac

Tagged releases include `paper-trail-ios-arm64-unsigned.ipa` and its `.sha256`
checksum. For development builds:

1. Push this source to GitHub.
2. Open **Actions → Unsigned iOS → Run workflow**, choosing the desired branch.
3. Download the `paper-trail-ios-unsigned` artifact.
4. Extract it and locate `paper-trail-ios-arm64-unsigned.ipa` and `SHA256SUMS`.
5. Use a sideloading tool such as iLoader to sign/install the IPA.

The IPA intentionally contains no signature, provisioning profile, Apple account,
notes, or credentials. A normal iPhone cannot run an unsigned application directly:
the sideloading tool must sign it. A free Apple account may work with your chosen
tool, subject to Apple's device/app limits and expiration rules. These limitations
are not bypassed by this project. No paid Apple Developer account or signing
secrets are required to **build** the workflow artifacts.

The workflow runs manually, on pull requests that change iOS/Rust build inputs,
or as a reusable job in the tag-triggered release workflow.
It has read-only repository permissions and never creates a release or tag itself.
It also produces an Apple Silicon simulator app archive and
runs a simulator smoke test. Workflow execution and physical-device/iCloud
behavior still need verification on Apple's platforms.

## Build on a Mac

Prerequisites: Apple Silicon macOS, Xcode 26.2 with iOS/simulator SDKs, Python 3.11+,
Rust stable (1.88+), and XcodeGen. Install prerequisites yourself; build helpers
do not install tools.

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
python3 build-aux/build_ios.py --dry-run
python3 build-aux/build_ios.py --output-dir dist/local
```

The helper generates Swift bindings and the Xcode project, builds both Rust
targets and both app bundles with signing explicitly disabled, and packages
fresh, non-overwriting output directories with SHA-256 checksums. Generated
bindings/project/Info.plist and intermediate builds are ignored by Git; the
project's source of truth is `ios/project.yml`.

The app's asset catalog includes an opaque 1024px icon rendered from the existing
Paper Trail SVG logo on a light background. Xcode generates the required device
sizes; iOS applies the corner mask. The committed PNG needs no image tools during
normal app/CI builds. To refresh it after changing the shared logo, use
ImageMagick with SVG support:

```sh
magick -background none -density 768 \
  data/icons/io.github.maennlse.paper-trail.svg -resize 1024x1024 \
  -background '#f5f6fa' -alpha remove -alpha off -colorspace sRGB -depth 8 -strip \
  PNG24:ios/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png
```

`build_local.py` remains the GTK/TUI/server **executable** packager. iOS uses a
separate helper because Xcode app bundles and unsigned IPA packaging are not
executable archives and cannot be built on the Linux host.

## Native glass design

The iOS 26 build uses Apple's Liquid Glass for floating controls, with native
navigation bars, menus, grouped forms, share sheets, and confirmation dialogs.
Glass stays in the navigation/control layer; the note canvas and list content
remain clear reading surfaces, without stacked blur or tinted text backgrounds.
The library has a floating search capsule between circular folder and **+**
controls. The editor uses **Done** and a **Note actions** menu, with a separate
save/language accessory. Sheets use the system presentation and drag indicator.

On iOS 17/18, floating controls use system material instead. Reduce Transparency
and Increase Contrast replace custom glass with opaque, outlined surfaces.
Reduce Motion disables custom panel/rail transitions; controls retain VoiceOver
labels and Dynamic Type. Native system components handle their own accessibility.
CI builds with Xcode 26.2, runs the full suite on iOS 26, and exercises Find,
sharing, and state behavior on an iOS 18 simulator as a fallback check.
Physical-device appearance, landscape, large text, and VoiceOver need manual review.

## Storage and UI behavior

- Default storage: `Documents/Paper Trail`, exposed in Files through
  `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`.
- Preferences can register a local/iCloud Drive folder through the native
  document picker and make it the default compose destination. Security-scoped
  bookmarks are native platform state, not shared note metadata.
- Folder access is file-coordinated. Bookmark failures are reported rather than
  silently replacing unavailable folders with an empty library.
- Folder tiles represent registered folders; All Notes aggregates their direct
  regular files, regardless of extension. Symlinks are excluded. Search checks
  filenames and full text across those folders. Non-UTF-8/NUL-containing files
  remain visible but cannot be opened in the editor, preventing lossy writes.
- Note-list search uses a floating search field at the bottom, next to the **+** New
  Note button. Queries search full contents across folders, including pinned notes.
  Each note shows its modification time at the upper-right: `HH:mm` today,
  abbreviated month/day this year, or `YYYY-MM-DD` for older years, matching GTK.
  A colored folder silhouette with initials replaces the visible folder name;
  the note overview and sidebar reuse the same badge and shared initials.
- The 76pt rail overlays a dimmed list without reflow. Open it with the sidebar
  button or a rightward left-edge swipe; dismiss with a scrim tap or left swipe
  on the scrim. Folder gestures are reserved for native drag and drop.
- Colored folder silhouettes, initials, and selection indicators follow GTK.
  Drag folders above/below another folder to reorder them. All Notes stays fixed;
  Move Up/Down context-menu and accessibility actions offer the same ordering.
  The order is persisted locally without changing the default compose folder.
- Notes autosave after a short debounce. Failed/conflicting writes retain the
  editor state and offer **Save a Copy** without overwriting the external edit.
- Swipe right on a note to pin/unpin, or left to rename/delete. Full swipes activate
  pin/unpin or the deletion confirmation; a cancelled deletion keeps the row.
  Buttons use the platform's native swipe appearance (rounding varies by iOS).
  Long-press offers the same actions. Rename changes the filename,
  not the first-line title; omitted extensions retain the original extension.
- Runestone provides the plain-text editor through a SwiftUI/UIKit bridge, with
  wrapping, native selection/undo, Dynamic Type, and the existing autosave and
  conflict protections. Its Swift package is pinned in `ios/project.yml`.
- The editor header shows the note's first-line title above its filename, using
  the same Rust title/fallback rules as GTK. Titles update while editing.
  **Done** or a rightward swipe from the left edge uses the same save-before-close
  path; failed saves keep the editor and draft open. A successful close requests
  sync when configured, signed in, and unlocked. Requests wait/coalesce behind
  active work; lock, sign-out, and cancellation clear queued automatic sync.
  Pulling down past the list's native refresh threshold also requests sync and a
  local refresh. Locked/local-only sessions still refresh without signing in or
  unlocking automatically. The rest of the canvas
  remains available for text selection and horizontal scrolling.
- Syntax highlighting uses pinned TreeSitterLanguages 0.1.10 grammars compatible
  with Runestone 0.5.2. Filename detection covers Markdown, JSON, Python,
  Rust, JavaScript, Swift, TOML, YAML, Bash/shell, HTML, CSS, and SQL, including
  uppercase extensions. Bash profile filenames such as `.bashrc` are recognized.
  The editor's **Language** button offers Automatic, Plain Text, and those
  bundled languages. Overrides use shared `note_language_overrides` metadata
  and survive rename/move/sync; GTK also recognizes explicit Plain Text.
  Unavailable desktop languages remain stored unchanged and display as plain
  text on iOS, with an explanation in the language chooser.
- Highlight colors adapt to light/dark appearance. Markdown inline syntax and
  fenced blocks in bundled languages are highlighted, not rendered.
  Syntax changes preserve the buffer, selection, and undo history; they do not
  enable bracket insertion, grammar-driven indentation, or other code-editing
  features.
- **Find** in the editor's **Note actions** menu (or Command-F on a hardware keyboard) opens a
  floating panel above the keyboard, with match count, options, close, and
  previous/next buttons. It uses Runestone's public search/highlight APIs rather
  than UIKit's inline find accessory. Options include match case, whole words,
  starts/ends with, and regular expressions. Invalid patterns are reported without
  changing the document. Pull the handle up (or tap it with VoiceOver) to reveal
  match previews; tapping a result selects and reveals it in the editor. Pull down
  to collapse without closing Find. **Replace** reveals single/all replacement
  controls, including regex capture templates such as `$1`. Replacements use
  Runestone's undo-aware editing APIs, and stale/read-only results cannot be edited.
  Closing Find restores the original selection, clamped to the current document.
  Finding alone does not change text or reset undo history. The save/sync
  footer is hidden while Find is open to avoid overlapping the panel.
- The editor distinguishes **Unsaved changes**, **Saving…**, **Saved locally**,
  and **Not saved**. Revision/attempt tokens prevent stale save results from
  describing newer edits as saved or failed. Failed writes keep the draft and
  offer **Retry** and **Save a Copy**.
- Sync feedback is separate from local saving: the library and Sync sheet show
  synchronization activity, actionable failures, and the last successful sync.
  The timestamp is device/profile-local and cleared on sign-out or confirmed
  reconfiguration. Failed, cancelled, or warning-bearing runs do not advance it.
  Ages show **Just now**, then minutes, hours, or days; they refresh at minute
  intervals, with no seconds display. **Last synced** sits at the bottom of the
  overview, below search; sync progress and actionable failures remain at the top.
- Long-press a note and choose **Move…** to select another registered folder.
  Contents, pins, and sync identity are preserved; filename collisions choose a
  unique name rather than overwriting either note. Open notes must be closed
  before moving.
- **Share…** in a note menu, or **Share** in the editor toolbar, prepares a
  temporary exported copy and explains that it is unencrypted. **Share File…**
  opens the native share sheet, including the iPad popover. **Share Text…** shares
  UTF-8 content directly instead of attaching a file; binary/NUL-containing notes
  offer file sharing only. The editor exports
  its current buffer even if saving failed, without modifying the original or
  its conflict baseline. List exports preserve original bytes, including binary
  files. Export directories are removed when their owning share screen is released.
- Long-press a folder for **Edit Folder…**, or use its Preferences entry.
  Editing changes its display label and color only, not its directory path,
  Files/iCloud bookmark, ordering, note pins, or sync mappings. Labels/colors can
  also be edited while a registered folder is temporarily unavailable.
  Display-name changes sync as logical folder metadata and survive subsequent
  syncs without renaming local directories. Competing device renames stop for
  explicit metadata confirmation rather than silently reverting the local edit:
  use **Sync Settings → Remote Folders → Review Name and Color**, confirm the
  remote name/color, then **Sync Now**. Rename again afterward if desired.
- Preferences live in the upper-right header; **+** New Note sits beside bottom
  search. The safe-area inset keeps the final note clear of the bottom controls.
  The rail's reload-style Sync menu offers Sync Now and Sync Settings;
  the icon rotates during synchronization (unless Reduce Motion is enabled).
- New folders have named color
  swatches and a selection checkmark in native form rows, rather than
  monochrome-tinted picker symbols. The existing saved color tokens are unchanged.
- Light appearance is the first-run default; light/dark/system and font size are
  configurable.
- Preferences follows GTK's **Show Line Numbers**, **Wrap Text**, and
  **Highlight Current Line** editor controls. The last option is the former
  “ruler” setting, not a vertical column guide. These controls use the same Rust
  settings and defaults as GTK.
- **Editor Font** offers System Default, Monospaced, and Custom Font through
  the native iOS font picker, with face selection, a preview, and font-size
  scaling that respects Dynamic Type. iOS PostScript font names are saved in
  `ios_custom_font`, separately from GTK's Pango `custom_font` description.
  A missing custom font falls back safely to the system font without discarding
  the saved name. Existing GTK font choices are not overwritten.
- Editor display changes update Runestone's view/theme without replacing its
  text or undo stack. Settings are committed together when **Done** is pressed
  and restored after relaunch.
- The **About** section shows the running app's bundle version/build and a native
  link to the GitHub source repository; the displayed version is not hardcoded.

## Rust API audit and boundary

- Titles, full-content search, sorting: `Note::from_text`, `Note::matches`,
  `note::sort_notes`.
- Portable native syntax catalog/detection: `syntax::{LANGUAGES,by_id,detect}`;
  per-note language choices use the existing `Settings::note_language_overrides`.
- Unsaved edits, debounce, conflict-safe saves: `Session::{edit,flush,open,close}`,
  `Store::save_if_unchanged`.
- Listing and file CRUD:
  `Repository::{list,load,create,rename,move_note,delete,save_if_unchanged}`.
- Atomic persistence: `papertrail_storage::atomic_write`.
- Folder names/colors/pins:
  `Settings::{folder_name,add_folder,set_pinned,rename_note,rename_folder}`,
  `colors::{normalize,hex}`.
- Metadata persistence: `SettingsFile::{load,save_merged}`.
- Sync configuration, worker, folder catalog: `control::Service`,
  `control::Command`, `Config`, `Service::folder_choices`.
- Native OAuth/PKCE: `auth::LoginFlow::{begin,complete}`, `Service::install_tokens`.
- E2EE: existing `papertrail-sync` engine and `papertrail-crypto`; not reimplemented.

### Sync folder management

In **Sync Settings → Remote Folders**, rows show the stable folder ID and
configured directory. Pending remaps also show the currently bound directory,
so duplicate names cannot conceal which folder owns a path.

**Download** explicitly selects that remote ID, clears its device-local exclusion,
and starts sync when signed in and unlocked. It does not implicitly accept a
metadata conflict or change mappings/other folder selections. Downloaded notes
are registered in the library when sync applies them.

Published folders have a trash button. Sign in and unlock first; deletion is
disabled while another operation or recovery-key confirmation is pending.
The native confirmation includes the folder name and ID, a destructive
**Delete from Server** action, and **Cancel**. Deletion permanently erases that
collection's server note history while retaining local files and unsaved edits
on every device. Server backups are outside this operation.

Deletion uses the shared independent erasure path, without syncing, flushing the
editor, applying mappings, or coordinating unrelated Files/iCloud folders.
The serial worker and cancellation/lock/sign-out handling still apply. The
sync sheet stays open and preserves unsaved connection/folder selections;
its remote rows refresh after the request, including after a lost response.
Opened retained deleted folders appear as **Local only** with no delete or download
button. Closed local folders and server-only deletion markers are not listed;
their bindings and deletion history remain intact. Retained files are not
automatically published again. iOS does not yet offer the desktop/TUI
**Sync as New Folder** action or a directory-remapping picker.

`crates/papertrail-ffi` exposes typed UniFFI records, errors, and a
`PaperTrailClient` object. Swift owns view state, file-provider coordination,
document bookmarks, and `ASWebAuthenticationSession`. It does not implement
search, filename normalization, pin/color metadata, conflict checks, encryption,
or the sync protocol. Blocking FFI calls execute on a serial native worker,
never the UI thread.

Settings use the existing desktop/TUI JSON schema, including path-keyed
`folder_colors`, `folder_labels`, and `pinned_notes`, with unknown-field-preserving
merged saves. Cross-device paths are **not** copied verbatim: the existing
encrypted folder catalog maps folder identities and pin/color metadata onto each
device's local paths. App-container and resolved-bookmark path changes use Rust
metadata rewriting and the sync relocation journal.

## Encrypted sync

Open **Sync → Sync Settings** from the folder rail and enter the **HTTPS server URL** plus upload
folder selections. The shared Rust client discovers `/v1/auth/config` and validates
the advertised OIDC provider. A new or changed provider requires explicit
**Trust and Save** confirmation before anything is persisted or authentication
starts. Cancelling, changing the draft, failed discovery, and stale confirmations
do not replace the saved profile. Existing bound account/vault identities cannot
be silently switched.

Account setup has no manual issuer, client ID, or scope controls. The server must
provide `/v1/auth/config`; failures require correcting/upgrading the server rather
than falling back to manual setup. Saved legacy profiles remain readable and usable;
reviewing setup again uses discovery without resetting accounts, vaults, or mappings.

Register this redirect URI with the OIDC provider:

```text
io.github.maennlse.paper-trail://oauth/callback
```

The existing Rust PKCE flow generates and validates the authorization request
and callback; Apple's web-authentication session only presents the browser.
Tokens use the existing credential-store implementation, extended to the iOS
Keychain via `keyring`'s Apple-native backend. Passphrases and recovery keys never
go in settings or build artifacts. Vault keys stay in the Rust sync controller.
Swift necessarily holds user-entered strings briefly for the FFI call; it clears
the secure field after submission and does not log secrets.

Like GTK, iOS shows one state-dependent **Sign In / Sign Out** action and one
**Lock Vault / Unlock Vault** action. Sync status and controls live in the rail's
Sync sheet; there is no duplicate lock button in the note-list header. Vault
creation is hidden for accounts known to have a vault, and sign-in defaults to
unlocking unless the server confirmed that no vault exists.

Create/unlock/change-passphrase share a native secret dialog. Creation and changes
require matching confirmation. Unlocking offers **Use Recovery Key Instead** and
recovery offers **Use Passphrase Instead**; switching clears the fields. An
explicit, initially-off **Remember encryption password on this device** toggle
uses the same Rust `Command::Passphrase` and account/vault-bound credential format
as GTK, backed by the iOS Keychain. Recovery keys are never remembered. Successful
remembered startup unlocks automatically sync; explicit locking suppresses
automatic unlock for the rest of that app session. Lock/sign-out requests made
during a worker operation are queued, cancel remaining work at safe checkpoints,
and cannot be undone by a late startup/login completion.

Save and acknowledge the recovery key before finishing vault setup. Cancelling
that step locks the vault and discards pending setup, as on GTK. Remote
folders can be selected for download. Sync runs while the app is open; this
version does not claim iOS background execution or iCloud-container entitlements.
Ordinary user-selected iCloud Drive folders use the Files provider instead.

## Verification

```sh
cargo test --locked -p papertrail-core -p papertrail-storage -p papertrail-crypto -p papertrail-sync -p papertrail-ffi
cargo clippy --locked -p papertrail-ffi --all-targets --all-features -- -D warnings
python3 -m unittest discover -s build-aux -p test_build_ios.py
```

The macOS workflow additionally compiles generated Swift, builds unsigned device
and simulator apps, and runs isolated UI tests plus state/export and Runestone
grammar unit tests (`PaperTrailStateTests`). Coverage includes stale-save completions,
retry/note-switch state, sync timestamp rules, export bytes/cleanup, moves,
folder editing, find/share preparation, syntax choices and parsing, pull-down
search, swipe-back, About, navigation, and persisted text edits.
Tests never use the user's notes or bookmarks.
Before distribution, manually test Files/iCloud provider availability, revoked
bookmarks, concurrent external edits, OAuth and recovery against a real server,
VoiceOver/Dynamic Type, and signing/installing with your chosen sideloading tool.
Also verify actual share destinations on iPhone/iPad (including cancellation),
Command-F and find dismissal/selection, and save/sync recovery while editing
offline or after an external conflicting edit.

### Readme screenshots

`testCaptureReadmeScreenshots` captures the actual library/rail and Runestone editor
with isolated demonstration files. It never reads real notes or bookmarks.
The macOS workflow exports its XCTest attachments as the
`paper-trail-ios-screenshots` artifact, including on successful runs.
Download it from **Actions → Unsigned iOS**; its manifest identifies the
`ios-library` and `ios-editor` PNG attachments. The readme includes real iPhone 16
Pro simulator captures from this test. To refresh them, review the new captures
and copy them to `data/screenshots/ios-library.png` and
`data/screenshots/ios-editor.png`. Do not substitute a mockup for a real
simulator/device capture.
