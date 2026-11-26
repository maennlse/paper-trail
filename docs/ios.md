# Native iOS client

Paper Trail for iOS is an iOS 17+ SwiftUI app backed by the same Rust note,
storage, metadata, crypto, and sync crates as the desktop and TUI clients.
Local notes remain UTF-8 `.txt` files. Sync encryption does **not** encrypt
local note files.

## Get an unsigned IPA without a Mac

1. Push this source to GitHub.
2. Open **Actions → Unsigned iOS → Run workflow**, choosing the desired branch.
3. Download the `paper-trail-ios-unsigned` artifact.
4. Extract it and locate `Paper-Trail-unsigned.ipa` and `SHA256SUMS`.
5. Use a sideloading tool such as iLoader to sign/install the IPA.

The IPA intentionally contains no signature, provisioning profile, Apple account,
notes, or credentials. A normal iPhone cannot run an unsigned application directly:
the sideloading tool must sign it. A free Apple account may work with your chosen
tool, subject to Apple's device/app limits and expiration rules. These limitations
are not bypassed by this project. No paid Apple Developer account or signing
secrets are required to **build** the workflow artifacts.

The workflow runs manually or on pull requests that change iOS/Rust build inputs,
has read-only repository permissions, and never creates a release or tag.
It also produces an Apple Silicon simulator app archive and
runs a simulator smoke test. Workflow execution and physical-device/iCloud
behavior still need verification on Apple's platforms.

## Build on a Mac

Prerequisites: Apple Silicon macOS, Xcode with iOS/simulator SDKs, Python 3.11+,
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

## Storage and UI behavior

- Default storage: `Documents/Paper Trail`, exposed in Files through
  `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`.
- Preferences can register a local/iCloud Drive folder through the native
  document picker and make it the default compose destination. Security-scoped
  bookmarks are native platform state, not shared note metadata.
- Folder access is file-coordinated. Bookmark failures are reported rather than
  silently replacing unavailable folders with an empty library.
- Folder tiles represent registered folders; All Notes aggregates their direct
  `.txt` children. Search always checks full note contents across those folders.
- The 76pt rail overlays a dimmed list without reflow. Open it with the sidebar
  button or a rightward left-edge swipe; dismiss with a scrim tap or left swipe.
- Notes autosave after a short debounce. Failed/conflicting writes retain the
  editor state and offer **Save a Copy** without overwriting the external edit.
- Long-press a note to pin/unpin or delete it. New folders have named color
  swatches and a selection checkmark in native form rows, rather than
  monochrome-tinted picker symbols. The existing saved color tokens are unchanged.
- Light appearance is the first-run default; light/dark/system and font size are
  configurable.

## Rust API audit and boundary

- Titles, full-content search, sorting: `Note::from_text`, `Note::matches`,
  `note::sort_notes`.
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

Open **Sync** from the folder rail and enter the **HTTPS server URL** plus upload
folder selections. The shared Rust client discovers `/v1/auth/config` and validates
the advertised OIDC provider. A new or changed provider requires explicit
**Trust and Save** confirmation before anything is persisted or authentication
starts. Cancelling, changing the draft, failed discovery, and stale confirmations
do not replace the saved profile. Existing bound account/vault identities cannot
be silently switched.

For older servers, expand **Advanced Authentication**, enable manual setup, and
enter the trusted issuer, public client ID, and scopes. Discovery mode is saved
with the profile, so authentication continues checking the server's advertised
settings against the accepted provider.

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
and simulator apps, and tests rail navigation plus a persisted plaintext edit in
an isolated test profile, never the user's notes or bookmarks.
Before distribution, manually test Files/iCloud provider availability, revoked
bookmarks, concurrent external edits, OAuth and recovery against a real server,
VoiceOver/Dynamic Type, and signing/installing with your chosen sideloading tool.
