# AGENTS.md

Instructions for AI coding agents working in this repository.

## This project

Rusty Wallpaper Gnome plays looping videos as the desktop wallpaper on GNOME
(Wayland), with a separate video for each monitor. GNOME has no
`wlr-layer-shell`, so a normal app cannot put itself behind the desktop. The
project therefore has two parts that only work together:

| Part | Path | Language | Job |
|---|---|---|---|
| Player | `src/` | Rust (GTK 4 + GStreamer) | Decodes videos and draws one borderless window per monitor |
| Shell extension | `extension/rusty-wallpaper@stunning7091.github.io/` | GJS (ES modules) | Spawns the player and turns its windows into desktop windows |

### How the parts talk

- **Launch:** the extension starts `rusty-wallpaper --managed` through
  `Meta.WaylandClient.new()` + `spawnv()`. Only windows from that client can be
  passed to `make_desktop()`, so the player must never be started any other way
  in normal use.
- **Window → monitor:** each player window is titled
  `rusty-wallpaper@<connector>` (for example `rusty-wallpaper@HDMI-1`). The
  extension reads the connector from the title and places the window on that
  monitor. `TITLE_PREFIX` is defined in both `src/desktop.rs` and
  `extension.js`; change both or neither.
- **Control channel:** the extension writes lines to the player's stdin.
  Today there is one command: `fullscreen <connector> 0|1`. EOF on stdin means
  the extension went away, and the player quits.
- **Lifecycle:** the extension restarts a crashed player after 2 s and gives up
  after 5 quick crashes in a row. Locking the screen disables the extension,
  which kills the player; unlocking starts it again.

### Source map

| File | What lives there |
|---|---|
| `src/main.rs` | Argument parsing, GStreamer/GTK start-up |
| `src/config.rs` | TOML config (`~/.config/rusty-wallpaper/config.toml`), path expansion, unit tests |
| `src/desktop.rs` | `Desktop`: windows per monitor, hotplug, config file watch, pause logic, stdin commands |
| `src/video.rs` | `Video`: one `playbin` pipeline per file, gapless looping, VA-API decoder ranking |
| `extension/.../extension.js` | Spawning, `make_desktop()`, placement, fullscreen detection, focus refusal, restart |
| `scripts/test-headless.sh` | End-to-end test in a throwaway headless GNOME Shell |
| `scripts/probe@rusty-wallpaper.test/` | Test-only extension: dumps window state + screenshots for the script above |
| `install.sh` | Builds and installs the binary, the extension and a starter config for the current user |

### Design decisions to keep

- **One decode per file.** Monitors that show the same file share one `Video`
  and its `gdk::Paintable`. Don't create a pipeline per window.
- **Gapless loop.** Looping uses non-flushing `SEGMENT` seeks on
  `SegmentDone`. A flushing seek on EOS is only the fallback; don't make it the
  main path, because it causes a visible hitch at the seam.
- **Zero-copy video.** `gst-plugin-gtk4` is compiled in (`dmabuf`, `wayland`,
  `gtk_v4_18` features), so VA-API frames reach GTK as DMA-BUFs. Don't switch
  to Debian's `gstreamer1.0-gtk4` package or add `videoconvert`/`glsinkbin`
  without measuring.
- **Video only.** `playbin` runs with `flags = "video"`; audio is never decoded.
- **Click-through.** Player windows clear their input region on every surface
  layout; the extension also drops focus if a player window ever gets it.
- **Config is the only settings surface.** No GSettings schema, no prefs UI,
  unless someone asks for one. The config reloads live on save.

### Target platform

- GNOME Shell 48 on Wayland is the only tested version (`metadata.json` lists
  only `"48"`). Mutter APIs change between releases: verify a method exists in
  the installed typelib before using it, and test before adding a version to
  `shell-version`.
- Hardware decoding goes through VA-API; `gst-inspect-1.0 va` lists the
  decoders available on the machine you test on.

## Working in this repo

### Commands

```sh
cargo build --release                  # player binary: target/release/rusty-wallpaper
cargo test                             # unit tests (config parsing)
cargo fmt
cargo clippy --release --all-targets   # must report zero warnings
scripts/test-headless.sh /tmp/rwg-test # end-to-end test, see below
./install.sh                           # install for the current user
```

Rust comes from rustup; in non-login shells `~/.cargo/bin` may not be on
`PATH`. Build dependencies (Debian): `build-essential pkg-config libgtk-4-dev
libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev`.

### Running the feature (required before "done")

**Never test in the user's real GNOME session.** On Wayland, GNOME Shell
cannot reload an extension in place: a broken extension means logging out.
Use the headless test instead:

```sh
cargo build --release && scripts/test-headless.sh /tmp/rwg-test
```

It starts a throwaway `gnome-shell --headless` with two virtual 1280×720
monitors, its own D-Bus session and its own XDG dirs, enables the extension
plus the probe, and runs four stages: `initial`, `fullscreen`, `restored`,
`mirrored`. Check all of the following:

1. `/tmp/rwg-test/probe/*.json`: each player window is `type: DESKTOP`, has
   `skipTaskbar: true` and `onAllWorkspaces: true`, its `rect` matches its
   monitor, and it never appears as `focus` or in `tabList`.
2. `/tmp/rwg-test/probe/*.png`: look at the screenshots. Each monitor shows
   its own video; `contain` letterboxes; `mirrored` shows the same frame twice.
3. `/tmp/rwg-test/shell.log`: `decoding with vah264dec` (hardware decode), and
   `Paused` then `Playing` around the fullscreen stage. There are no `JS ERROR`
   lines.

If you change behaviour the script doesn't exercise yet, extend the script and
probe instead of testing by hand.

Player-only changes can be eyeballed with
`cargo run --release -- --windowed --config <file>`. That opens normal windows
in the current session and touches nothing else.

### Rust conventions

- Edition 2024, `rustfmt` defaults, zero clippy warnings.
- Crates are renamed in `Cargo.toml`: use `gtk::…` and `gst::…`.
  `gtk::{gdk, gio, glib, cairo}` come from the `gtk` re-exports; don't add
  them as separate dependencies.
- Shared state is `Rc<RefCell<Desktop>>`. Signal handlers capture
  `Rc::downgrade` and `upgrade()` inside, so closures never keep `Desktop`
  alive.
- Errors: functions that can fail return `Result<_, String>` with the file
  path in the message. The caller logs it with an `eprintln!` prefixed
  `rusty-wallpaper:` and keeps running. The player must survive a missing
  config, a missing video file or a broken video; a crash loop makes the
  extension give up.
- Logging goes to stderr only, which ends up in the GNOME Shell journal. Keep
  routine output to about one line per video start. Chatty diagnostics go
  behind `RUSTY_WALLPAPER_DEBUG`.

### Extension conventions

- GNOME 45+ ES-module style: `export default class … extends Extension`, with
  imports from `gi://…` and `resource:///org/gnome/shell/…`.
- Everything `enable()` creates, `disable()` undoes: every `connect()` gets a
  `disconnect()`, every `GLib.timeout_add` gets a `GLib.source_remove`, and
  the player process is killed. This is also a rule of extensions.gnome.org
  review.
- Check `this._windows`/`this._client` before using them in callbacks; a
  callback can fire after `disable()`.
- Use `console.error/warn` with `${this.uuid}:` as the prefix.

### Git

- Commits in this repo use the GitHub noreply identity set in the local git
  config. Don't change it, and don't put the owner's personal email anywhere
  in the repo.
- The repository is public: no secrets, local paths or personal data in
  commits, test fixtures or logs.

## Priority order

When decisions conflict, resolve them in this order:

1. **Functionality** — the app must do what it is supposed to do. Correct behavior is the baseline; nothing below may break it.
2. **Clean** — readable, well-named, self-documenting code that another developer (or you, in six months) understands without a walkthrough.
3. **Lean** — no code, dependency, abstraction, or feature that does not earn its place.
4. **Efficient** — sensible use of CPU, memory, network, and battery; fast startup and responsive UI.

If a tradeoff is unavoidable, the lower-numbered priority wins. Never sacrifice correctness for elegance, or elegance for micro-optimization.

## Core philosophy: simplicity is the goal, not the starting point

> "An idiot admires complexity, a genius admires simplicity." — Terry A. Davis

- Write the simplest solution that fully satisfies the requirement — then stop.
- Every line of code is a liability. If you can delete it and nothing breaks, delete it.
- If a function, config, or abstraction exists "just in case," remove it. Add things when the need is real, not when it is imaginable.
- The whole module should be understandable by one person in one sitting. If it isn't, split it or simplify it.

## Lean rules

- **Dependencies:** every new dependency must be justified in the PR/commit message. One well-understood function does not justify a package — prefer a small local implementation over a heavyweight library.
- **No premature abstraction:** no interfaces, generic layers, factories, or plugin systems until there is a second real use case. YAGNI applies always.
- **No dead surface:** remove unused exports, commented-out code, unused imports, disabled feature flags, and placeholder routes immediately — do not leave them "for later."
- **Features:** implement exactly what is asked. Do not add settings, telemetry, dark-mode toggles, or admin panels nobody requested.
- **Files/config:** keep the repo tree flat where possible. No config for things that have sensible defaults.

Current dependencies and why they exist:

| Crate | Why |
|---|---|
| `gtk4` | Windows, monitor list, rendering of the video paintable |
| `gstreamer` | Decoding and looping |
| `gst-plugin-gtk4` | Video sink compiled in, so DMA-BUF zero-copy works without a distro package |
| `serde` + `toml` | Config file |
| `async-channel` | Hands stdin lines from the reader thread to the GTK main loop |

## Clean rules

- **Names:** descriptive and boring. `userLoginAttempts`, not `ula` and not `IManageUserAuthenticationService`. If you need a comment to explain the name, rename it.
- **Functions:** one job each. If you need "and" to describe it, split it. Short functions; early returns over nested conditionals.
- **Comments:** explain *why*, never *what*. Delete comments that restate the code.
- **State:** minimize mutable and global state. Make data flow obvious; no hidden side effects.
- **Errors:** handle them at the level where recovery is possible. No empty catch blocks, no swallowing failures silently.
- **Consistency:** follow the existing conventions of this codebase over your personal preferences. Match file structure, naming, and formatting of neighboring code.

## Efficient rules

- Default to the straightforward implementation; optimize only with a measured reason (profiler, benchmark, or user-visible latency).
- Choose appropriate data structures upfront (hash map for lookups, not array scan) — that is design, not premature optimization.
- I/O is the usual bottleneck: batch requests, avoid redundant network calls, cache when the same data is fetched repeatedly.
- Startup and critical paths stay lazy: load what's needed now, defer the rest.
- On the UI: don't block the main thread; don't leak listeners, subscriptions, or timers.
- Mobile/web: treat battery and bandwidth as user resources, not as free.

For this project in particular:

- A wallpaper runs all day; idle cost is what matters. Paused or hidden
  monitors must not decode. Keep the hardware decode path, and don't add
  per-frame work on the CPU.
- Code in `extension.js` runs inside GNOME Shell's main thread: anything
  blocking there freezes the whole desktop. Only tiny pipe writes are
  acceptable; anything bigger must be async.

## Resource and memory correctness

Every change must be checked for leaks and resource errors, not just for whether it compiles:

- **Memory leaks:** every allocation has a matching release. In managed languages, this means no accidental retention: remove references, clear caches, and null out large buffers when done.
- **Listeners, subscriptions, timers, observers:** every `on`/`addEventListener`/`subscribe`/`setInterval`/`requestAnimationFrame` has a matching cleanup (`off`, `removeEventListener`, `unsubscribe`, `clearInterval`, `cancelAnimationFrame`). Verify this on every component teardown, route change, and error path.
- **Handles and streams:** files, sockets, database connections, and streams are always closed — including on exceptions (use `finally`, `defer`, `with`, or RAII as the language provides).
- **Error paths leak too:** a leak that only happens when an error is thrown is still a leak. Test the failure branch, not only the happy path.
- **Run the actual checkers** available for the stack before finishing: linters, leak detectors, sanitizers (e.g. Valgrind, AddressSanitizer/LeakSanitizer, heap profilers), or devtools memory profiling. Do not claim "no leaks" from reading the code alone when a tool can verify it.

Where resources live in this project:

- **GStreamer pipelines:** `Video`'s `Drop` sets the pipeline to `Null`; the
  `BusWatchGuard` field removes the bus watch. `Desktop::sync()` drops videos
  no monitor uses any more. A new pipeline or watch must follow the same
  pattern.
- **GTK windows:** removed outputs call `window.destroy()`. Keeping a
  `gtk::Window` in a struct does not destroy it when the struct drops.
- **GLib signal handlers in Rust:** handlers on long-lived objects
  (`display.monitors()`, the config `FileMonitor`) capture only weak
  references. A strong `Rc` in a handler is a cycle.
- **Extension:** see "Extension conventions". Also check the player-exit path
  (`_onPlayerExited`) and the `disable()` path; both must leave no timers,
  signals or processes behind.
- **Checking:** Valgrind and heaptrack are not installed on the development
  machine. Install them if you need them, or at minimum watch the player's
  memory across many loop cycles and config reloads:
  `watch -n5 'ps -o rss=,cmd= -C rusty-wallpaper'`. RSS must level off, not
  keep growing.

## Definition of done

Before declaring a task complete, verify all of the following:

1. All existing and new tests pass; new behavior has tests.
2. **You actually ran the feature and confirmed it works** — execute the app, click the flow, call the endpoint, or run the script that exercises the change. "It should work" is not done; "I ran it and it works" is.
3. **Memory and resource check:** no leaks, dangling listeners, unclosed handles, or unbounded growth — verified with the tooling named in the section above where available.
4. The build compiles/Type-checks with zero new warnings.
5. `git diff` contains nothing unrelated to the task — no reformatting of untouched files, no drive-by refactors.
6. No dependency was added without justification; none removed silently.
7. A new team member could read the changed code and explain what it does without help.
8. You have looked at every line you wrote once more and deleted whatever you can.

In this repo, items 1–4 mean: `cargo test`, `cargo clippy --release
--all-targets` with zero warnings, and `scripts/test-headless.sh` with all
three checks from "Running the feature" passing. If you change the config
format, the stdin protocol or the install steps, update `README.md` and
`config.example.toml` in the same change.

## When requirements are unclear

Implement the smallest interpretation that satisfies the literal request, make assumptions explicit in code comments or commit message, and leave the extension points out. Do not guess at elaborate requirements.
