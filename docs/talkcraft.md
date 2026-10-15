# TalkCraft: what the fork adds, and what we learned

TalkCraft is a GERBER.MEDIA fork of SoundCraft for podcasts. The engine is SoundCraft's, unchanged
where possible; the fork adds podcast commands, a slimmer interface and remote guests. This page
is for whoever works on it next (people and agents): where things live, why they are built the way
they are, and the traps already stepped in. `AGENTS.md` still holds for everything else.

Last updated: 2026-10-14 (Settings tabs).

## Where things live

| Area | Files |
|---|---|
| Podcast commands (import remote, sync, extend clip, ducking, export…) | `crates/engine/src/cmd/podcast.rs` |
| Menu filtering, app name, hidden commands | `crates/ui-egui/src/podcast.rs` |
| Work modes (Record / Edit / All) | `crates/ui-egui/src/workspace.rs` |
| Remote guests: studio server, window | `crates/ui-egui/src/remote.rs`, `remote/host/index.html`, `remote/guest/index.html` |
| Pitch-preserving playback rate (WSOLA) | `crates/playback/src/stretch.rs` |
| History panel | `crates/ui-egui/src/panels.rs` (`undo_history`), `Engine::history_go` |
| Version number | `packaging/version.env`, `crates/engine/build.rs`, `packaging/env.sh` |
| German interface | `crates/ui-egui/src/i18n.rs` (English text is the key) |

Crate names stay `soundcraft-*` and the Cargo version stays SoundCraft's, so upstream merges stay
simple. Only the apps are called `talkcraft`.

## Decisions

- **Everything is still a command.** New behaviour goes into `crates/engine/src/cmd/*` with a test.
  UI-only commands (`ui.workspace`, `transport.rate`, `window.remote`) live in `menus.rs`
  `UI_COMMANDS` and are dispatched in `extra_windows.rs`.
- **Work modes only hide, never change.** `workspace.rs` filters menus, shortcuts, toolbar groups and
  track-header buttons. The session and every command behave the same in every mode, so switching
  never loses anything. The History panel shows only while editing (and in All).
- **Track groups are hidden** (`HIDDEN_COMMANDS`, the Groups list appears only for sessions that
  already have groups). Alt already edits every track, and deleting closes the gap on all tracks.
- **A Grabber click places the insertion point;** dragging moves the clip, Shift/Cmd-click or a
  double-click selects it. At a clip edge every pointer tool trims (cursor ↔).
- **`edit.extend_clip` brings back exactly what was cut.** Pulling an edge stops at the recording's
  end, at the next piece of the same recording (that audio already plays there) and, without
  ripple, at the neighbouring clip. Every partner clip on other tracks (Alt) gets the same limits.
  In Shuffle the rest of the track moves along.
- **Playback rate keeps the pitch** (WSOLA between the mix and the device). It applies only while
  listening back, never while recording or monitoring an input: a take must land where it was
  played, and the input ring must not run dry.
- **Remote guests run browser to browser.** The guest page is static (Codefloe Pages), PeerJS only
  sets up the connection. The studio page is served by TalkCraft on `127.0.0.1` with a random port
  and a random path token, so no other web page can reach it. Guests record lossless WAV in
  their browser (AudioWorklet, 16-bit right away for phones); the studio keeps a muted backup of the
  call and `podcast.import_remote` syncs the guest's file onto it by cross-correlation.
- **Guests are told apart by name** (their track is "Ferngast – Name"); the studio numbers
  duplicates ("Anna 2"). Track names are German on purpose, like the template's.
- **Settings, not menus, for what is set once.** Setup > Preferences has tabs (Episode, General,
  Appearance, Audio, Transcription). Appearance and the former Options menu moved there; the Options
  menu is gone (Loop Playback, still switched often, sits under View). Each check box runs the menu
  command it replaced, so behaviour is unchanged (`dialogs.rs`: `GENERAL_OPTIONS`,
  `APPEARANCE_OPTIONS`, `AUDIO_OPTIONS`; a test switches every one of them). Most of these options
  are saved with the session, and the tabs say so.
- **Switches that do nothing are not offered.** Many Options/View items of the incumbent only store
  a value nothing reads (markers follow edit, solo modes, low-latency monitoring, several clip and
  marker displays …). They are hidden (`WITHOUT_EFFECT`), not moved; before offering a switch, grep
  whether anything reads its flag.
- **Version:** `TALKCRAFT_MINOR.<commits since TALKCRAFT_FORK_BASE>` (`packaging/version.env`).
  Raise `TALKCRAFT_MINOR` by hand for bigger steps; the last number counts itself.

## German interface

`crates/ui-egui/src/i18n.rs` holds the table (the English text is the key). `tr("…")` for fixed
text, `trf("… {} …", &[&a, &b])` for text with values (the German template keeps the same `{}`
in the same order). Widgets that take a tooltip or title (`icon_button`, `text_toggle`,
`rec_toggle`, `panel_header`, `windows::win`, dialog titles) translate it themselves, so callers
pass English. Do not run names (tracks, clips, plugins) through `tr`: a track called "Record"
would turn into "Aufnahme". Pro Tools terms the German Pro Tools also keeps (Shuffle, Slip, Grid,
Nudge) stay.

`crates/ui-egui/tests/german_complete.rs` keeps it complete: one test opens every menu, window,
dialog and work mode in German and fails on any string without a German entry; the other checks
every `tr`/`trf` string in the code, including tooltips and messages the walk never shows.

## Traps we stepped in

### macOS menu bar (muda)
`MenuEvent::set_event_handler` stores the handler in a `OnceCell`: only the **first** call counts.
The bar is rebuilt when the work mode changes; a handler bound to the old bar's channel silently
dropped every menu click ("all buttons dead"). Fix: install the handler once and route through a
static to whichever bar is current (`apps/soundcraft/src/native_menu.rs`, `ROUTE`).

### Path prompts that look like nothing happened
File > Import used to open a path field pre-filled with the home folder; OK tried to import a
folder and only the status line said so. Opening and importing now go straight to the system file
dialog (`menus::pick_directly`), and failures open a message.

### Memory with long episodes
An hour of 48 kHz mono is about 173 M samples. Anything that collects per-frame data over a whole
episode explodes: the denoiser kept every FFT frame (about 5.5 GB per hour), and the loudness
readout rendered the whole mix into memory after every edit (about 1.4 GB per hour of stereo).
Both now stream: the denoiser recomputes spectra in its second pass (bit-identical, tested against
the old version), and `soundcraft_mix::render_range_each` feeds the meter block by block.
Rule: never hold `O(episode length × something)` beyond the audio itself.

### Locks the UI thread also takes
The studio server's shared state is locked by the UI every frame. Never do slow work (writing an
upload) while holding it.

### Device watching
A freshly opened input reports zero callbacks until its first buffer. The stall clock must start
when an input opens and when a take starts, or a slow USB/Bluetooth input stops the take at once.
Replacing the Player (device lost, reconnected, picked) must re-attach the open input's monitor
ring (`SoundApp::replace_player`), or input monitoring goes silent.

### Menu check marks
`menus::checked_state` knows the `options.*` booleans but not the view flags (`edit.flag(...)`) or
the scrolling mode, so those menu items never showed a check mark. The Settings read the state
directly (`option_state`); a choice (scrolling, Peak/Power) must always show exactly one entry.
Some flag commands only turn a flag on (`view.clip_time_current`); turning it off goes through
its counterpart (`No Time`).

### Range markers and cutting time
`edit::cut_time` must shorten a range marker that runs into or across the cut, not only move the
markers after it.

### Build scripts
Cargo treats a `rerun-if-changed` path that does not exist as always changed, which rebuilt the
engine and everything above it on every build. Watch only files that exist.
CI checks out shallowly by default; counting commits since the fork then fails. The packaging
scripts unshallow, and refuse to build rather than ship a made-up version.

### Browsers
- Chrome feeds a remote WebRTC stream into Web Audio only while a media element plays it (muted).
- iPhones play sound only if playback started inside a tap: create and `play()` the element in the
  click handler, attach the stream later.
- Phones stop the microphone when the screen locks: request a Wake Lock and warn if the page went
  to the background while recording.
- An `AudioContext` created without a user gesture is suspended: the studio page asks for a click.
- Muting a cloned track (`enabled = false`) works, but the meters must follow, or it looks like the
  microphone is still open. The guest also replaces the sender track with `null`.

## How we test what tests cannot see

- **Offscreen UI:** `cargo run -p soundcraft-ui-egui --example ui_shot -- out.png script.jsonl`
  renders the real UI from a script of control requests (`{"shot": "…"}` takes a picture).
  It has no file dialogs; add stub `Services` in a temporary example to see Browse buttons.
- **Running app:** `talkcraft --control 0` prints a port; JSON lines drive it (`ui.click`,
  `ui.screenshot`, `engine.execute`, `ui.menu.invoke`).
- **Real menu clicks (macOS):** `osascript` with System Events can click items of the app's menu bar
  (needs the Accessibility permission). `ui.menu.invoke` skips muda, so it would not have found the
  menu-handler bug.
- **Is the app stuck in a modal dialog?** `sample <pid> 1` and look for `runModal`.
- **Remote guests end to end:** a small Python server that serves `remote/host` (placeholders
  replaced) and `remote/guest`, and Playwright with two Chromium contexts and a fake microphone
  (`--use-fake-device-for-media-stream`), connected through the real PeerJS service. Measure what
  arrives at the studio with an `AnalyserNode` on the guest's stream.
- **Responsive pages:** Playwright with a phone viewport; headless Chrome alone does not render
  responsively.

## The guest page

The source is `remote/guest/` in this repository. It is published from the separate public
repository `GerberMedia/Talkcraft-Gast`, branch `pages`
(<https://gerbermedia.codefloe.page/talkcraft-gast/>): copy `remote/guest/*` there and push.
The Pages path is lower case. `remote::GUEST_PAGE` must match the published address.

## Audit, 2026-10-14

A review of everything since the fork found ten problems; all are fixed with tests where a machine
can check them (commit `5d1b44a`): playback rate during recording, denoiser and loudness memory,
a false "input disconnected", silent monitoring after an output change, partner clips overlapping
in Slip, the UI hanging during uploads, range markers not shortening, constant rebuilds, and a
wrong version from shallow clones. The device cases (false stall, monitoring after an output
change) need a real interface to confirm.

Open: the denoiser still runs on the UI thread (the app pauses during long clips); moving it to a
background job is a separate step.

## Publishing (`crates/engine/src/publish/`)

- No HTTP stack in the app: requests go through the system's `curl` (macOS, Linux, Windows 10+)
  behind a `Transport` trait, so tests check what would be sent with a fake. Pre-signed upload URLs
  carry secrets in the query: error messages strip it.
- macOS `curl` has no sftp/scp: SFTP runs the system `sftp -b -` with `BatchMode=yes` (key login
  only, never a hidden password prompt). Git runs with `GIT_TERMINAL_PROMPT=0` for the same reason.
- Keys live in `<settings>/publish/publish.json` (mode 0600), never in the session or `ui.json`:
  sessions get shared. TalkCraft ships no keys.
- Own feed: the list of episodes is kept in `<settings>/publish/feeds/<show>.json` and the whole
  `feed.xml` is rewritten each time; a re-publish of the same episode (same guid) keeps its date.
- Hosts checked against their own docs or OpenAPI: Buzzsprout, Transistor, Podigee, Podbean,
  Spreaker, Captivate (via its official npm client), RSS.com. Castos and Megaphone come from
  secondary sources and take audio only as a URL, so the file goes to the user's storage first;
  the UI marks them untested. Simplecast, Blubrry and Libsyn have no public upload API.
- None of it has run against a live service (no accounts). When a user reports a failure, the log
  in the window shows the HTTP status and the start of the answer.
- The interface font has no ✓ (U+2713): it renders as a box. Use ✔ (U+2714, in egui's emoji
  font) or draw a shape.

## Fallback relay for remote guests

- PeerJS's default ICE servers include its own TURN, but on port 3478 only (UDP/TCP). Company
  networks that pass only web traffic block it: ICE stays at "checking" (seen behind Cisco
  AnyConnect). Only a TURN server on 443 (`turns:…:443?transport=tcp`) gets through.
- No free public TURN is left (Open Relay's `openrelay.metered.ca` resolves to 192.0.2.1), so the
  user enters their own in the Remote Guests window (`UiState.turn`). It travels in the invitation
  link's `#` part (`#ROOM&turn=<json>`), never sent to the Pages server; TURN credentials are
  visible to guests by nature.
- The guest page tries without it first and switches after 12 s without an open data connection;
  the studio page offers it from the start (direct paths still win). Tested with Playwright in a
  network where the direct path hangs: switch after 12 s, then the "not even through the fallback"
  hint. A working relay could not be tested (no account).

## Palettes and kept settings

- `palettes.rs` shows Jingles, Run Sheet and Remote Guests either as an `egui::Window` or, detached,
  through `show_viewport_immediate` (a real OS window). The builder's position and size are passed
  only from the state saved when the window first opens in this run (`ExtraState.palette_start`):
  feeding back the live position every frame fights the user's dragging. Backends without
  multi-viewport support (`ctx.embed_viewports()`) fall back to the in-app window.
- Input in a detached window (dropped files, hover) belongs to that viewport: read it from the
  body's `ui.ctx()`, not from the main context. The main window's keyboard shortcuts (1–8 for
  jingles) do not fire while the detached window has the focus; clicks and touches do.
- `palette.rs` (no s) is the command palette. Do not mix the two up.
- Working preferences live in the session's `EditState` (as in the incumbent) but users expect
  them to stay: `prefs.rs` copies them to `UiState.edit_prefs` when they change and applies them to
  every document `replace_session` brings (`Engine.generation`), keeping a fresh session clean.
- `ui.json` is written a second after any change (status line excluded), not only on quit.
- ✎ (U+270E) is also missing from the interface font. Prefer words over symbols.

## Interface font and icons

- The interface font is Inter (bundled, OFL). Helvetica Neue from the system rendered wavy in egui
  at 1× scale (non-Retina screens): glyphs of small text jumped up and down. Compare renders with
  `UI_SHOT_PPP=1` for the `ui_shot` example, since at 2× it looks fine.
- `icons::draw` draws into the largest centred square of the rect it gets: the transport buttons
  are wider than tall and stretched the stop square into a bar.
- Jingle pads: any number from 1 to 32 (`PodcastSettings.jingle_pads`, 0 = eight), a picture per
  pad (`jingle_images`), `podcast.jingle_pad {add|remove}`. Keys 1–8 still fire the first eight.

## On Air, remote control, sorting

- `onair::status` is the one source for the On Air palette, the `studio.status` command and the
  phone page, so they never disagree.
- The remote control (`remote_control.rs`, page `remote/control/index.html`, compiled in) listens on
  all interfaces under a random token and runs only while switched on. The address shown is the
  interface of the default route (UDP connect trick, no packet sent). Company laptops may drop
  inbound connections entirely (seen here: even `python3 -m http.server` was unreachable on the LAN
  address with the macOS firewall off), so tests use 127.0.0.1 with the same token.
- `reorder::Reorder` sorts any list by a grip: the order is previewed live while dragging and
  applied once on release (one undo step). Places are remembered per slot from the last frame.
  The jingle test drives a real drag through `egui::Context::run_ui`.
- `fonts::tests::every_symbol_in_the_interface_has_a_glyph` fails when a string literal uses a
  character no bundled font has (✓, ✎, ✕ and ♯ showed as boxes before).
