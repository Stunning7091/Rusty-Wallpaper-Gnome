# Rusty Wallpaper Gnome

Looping video wallpapers for GNOME on Wayland, with a different video on each monitor.

Wallpaper Engine is Windows-only, and most Linux live-wallpaper tools rely on the
`wlr-layer-shell` protocol, which GNOME doesn't implement. This project gets
around that with two small parts:

| Part | Language | Job |
|---|---|---|
| `rusty-wallpaper` | Rust (GTK 4 + GStreamer) | Decodes the videos and draws one borderless window per monitor |
| Shell extension | JavaScript | Starts the player and turns its windows into real desktop windows |

## How it works

```
GNOME Shell ── extension ──spawns──▶ rusty-wallpaper --managed
    │              │                     │
    │              │  make_desktop()     ├─ one GStreamer pipeline per video file
    │              │  stick(), place     │    (VA-API decode → DMA-BUF → GTK, no CPU copies)
    │              │  on its monitor     └─ one window per monitor, titled
    │              │                          rusty-wallpaper@<connector>
    │              └─ stdin: "fullscreen HDMI-1 1"  → pause that monitor's loop
```

- The extension launches the player through `Meta.WaylandClient`, which lets it
  recognise the player's windows and call `make_desktop()` on them. Desktop
  windows sit below everything, are skipped by Alt+Tab and the overview, and the
  extension pins each one to the monitor named in its title.
- The windows have an empty input region, so clicks go to the desktop.
- Videos loop with non-flushing segment seeks, so there's no gap or black frame
  at the seam.
- If two monitors use the same file, it is decoded once and shown in both windows.
- The overview (Super key) draws its own static wallpaper previews. The
  extension puts live clones of the video windows on top of them, so the video
  keeps playing there too.
- A monitor's video pauses while a fullscreen window covers it.
- The config file is watched and reloads as soon as you save it. Monitor
  hotplug is handled too.
- If the player crashes, the extension restarts it. After 5 quick crashes in a
  row it gives up.

## Requirements

- GNOME Shell 48 on Wayland (the only version tested so far)
- Rust (via [rustup](https://rustup.rs))
- Build dependencies, on Debian 13 / Ubuntu:

  ```sh
  sudo apt install build-essential pkg-config libgtk-4-dev \
      libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
  ```

- Runtime: GStreamer's `plugins-good` (MP4/WebM demuxers) and `plugins-bad`
  (VA-API decoders), which most GNOME desktops already have. The GTK 4 video
  sink is compiled into the binary, so `gstreamer1.0-gtk4` isn't needed.

## Install

```sh
git clone https://github.com/Stunning7091/Rusty-Wallpaper-Gnome.git
cd Rusty-Wallpaper-Gnome
./install.sh
```

This installs `~/.local/bin/rusty-wallpaper`, the extension, and a starter
config at `~/.config/rusty-wallpaper/config.toml`. On Wayland, GNOME only picks
up new extensions after you log out and back in. After that:

```sh
gnome-extensions enable rusty-wallpaper@stunning7091.github.io
```

## Configure

```toml
# ~/.config/rusty-wallpaper/config.toml
[monitor.HDMI-1]
file = "~/Videos/wallpapers/ocean.mp4"

[monitor.HDMI-2]
file = "~/Videos/wallpapers/forest.mp4"
fit = "contain"   # cover (default) | contain | fill

[pause]
fullscreen = true
```

`rusty-wallpaper --list-monitors` prints the connector names. A `[default]`
section applies to any monitor without its own entry. See
[`config.example.toml`](config.example.toml) for every option.

**Good loop files:** H.264, HEVC, VP9 or AV1 at your monitor's resolution, cut
so the last frame flows into the first. Audio tracks are ignored.

## Try the player without the extension

```sh
cargo run --release -- --windowed --config config.example.toml
```

This opens ordinary windows, which is handy for checking that a video loops
cleanly.

## Test without touching your session

```sh
cargo build --release && scripts/test-headless.sh /tmp/rwg-test
```

This starts a throwaway headless GNOME Shell with two virtual monitors, then
checks placement, fullscreen pause and mirroring. It saves screenshots and
window state to `/tmp/rwg-test/probe/` and the logs to `/tmp/rwg-test/shell.log`.

## Troubleshooting

```sh
journalctl --user -f -o cat | grep -i rusty         # extension + player messages
GST_DEBUG=2 rusty-wallpaper --windowed             # GStreamer warnings
gst-inspect-1.0 va                                 # are VA-API decoders available?
```

## Known limitations

- In the overview (Super key) the video previews have square corners, where
  GNOME's own wallpaper previews are rounded.
- GNOME changes its extension APIs between releases, so other Shell versions
  will need testing before they're added to `metadata.json`.

## License

MIT
