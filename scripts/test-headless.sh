#!/usr/bin/env bash
# Smoke test in a throwaway headless GNOME Shell with two virtual monitors.
# Installs nothing and leaves your real session and settings alone: the shell
# gets its own D-Bus session, config/data dirs and Wayland socket.
#
# Usage: scripts/test-headless.sh [WORK_DIR]
# Produces WORK_DIR/probe/{initial,fullscreen,restored,mirrored,workspace-1-mid,
# workspace-1,workspace-0-mid,workspace-0,overview,disabled,reenabled}.{png,json}
# and WORK_DIR/shell.log (shell, extension and player output).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
uuid=rusty-wallpaper@stunning7091.github.io
probe=probe@rusty-wallpaper.test

if [ -z "${RWG_INNER:-}" ]; then
    work=$(realpath -m "${1:-$(mktemp -d)}")
    rm -rf "$work"
    mkdir -p "$work"/{config/rusty-wallpaper,data/gnome-shell/extensions,cache,probe,videos}
    cp -r "$repo/extension/$uuid" "$repo/scripts/$probe" "$work/data/gnome-shell/extensions/"

    ffmpeg -loglevel error -y -f lavfi -i testsrc2=size=1280x720:rate=30 -t 3 \
        -c:v libx264 -pix_fmt yuv420p "$work/videos/left.mp4"
    ffmpeg -loglevel error -y -f lavfi -i smptehdbars=size=960x720:rate=30 -t 3 \
        -c:v libx264 -pix_fmt yuv420p "$work/videos/right.mp4"

    unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET
    export RWG_INNER=1 RWG_WORK=$work
    export XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data XDG_CACHE_HOME=$work/cache
    export RUSTY_WALLPAPER_BIN=$repo/target/release/rusty-wallpaper RUSTY_WALLPAPER_DEBUG=1
    export RWG_PROBE_DIR=$work/probe
    exec dbus-run-session -- "$0"
fi

work=$RWG_WORK
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell enabled-extensions "['$uuid', '$probe']"
gsettings set org.gnome.shell welcome-dialog-last-shown-version '999'

display=rwg-test-$$
gnome-shell --headless --wayland --no-x11 --wayland-display "$display" \
    --virtual-monitor 1280x720 --virtual-monitor 1280x720 >"$work/shell.log" 2>&1 &
shell=$!
fullscreen_app=
trap 'kill $fullscreen_app $shell 2>/dev/null; wait $shell 2>/dev/null || true' EXIT

for _ in $(seq 100); do
    [ -S "$XDG_RUNTIME_DIR/$display" ] && break
    sleep 0.1
done
export WAYLAND_DISPLAY=$display

snap() {
    touch "$RWG_PROBE_DIR/request-$1"
    for _ in $(seq 50); do
        [ -s "$RWG_PROBE_DIR/$1.json" ] && [ -s "$RWG_PROBE_DIR/$1.png" ] && { echo "captured $1"; return; }
        sleep 0.2
    done
    echo "probe did not answer for $1" >&2
    return 1
}

"$RUSTY_WALLPAPER_BIN" --list-monitors | tee "$work/monitors.txt"
mapfile -t connectors < <(cut -f1 "$work/monitors.txt")

# The player is already running (started by the extension with no config);
# writing the config makes it pick the videos up live.
write_config() {
    cat >"$XDG_CONFIG_HOME/rusty-wallpaper/config.toml" <<EOF
[monitor.${connectors[0]}]
file = "$work/videos/left.mp4"

[monitor.${connectors[1]}]
file = "$work/videos/$1"
fit = "contain"
EOF
}
write_config right.mp4
sleep 4
snap initial

gjs -c '
imports.gi.versions.Gtk = "4.0";
const {Gtk, Gdk, GLib} = imports.gi;
Gtk.init();
const window = new Gtk.Window({title: "fullscreen-test"});
window.fullscreen_on_monitor(Gdk.Display.get_default().get_monitors().get_item(0));
window.present();
new GLib.MainLoop(null, false).run();
' &
fullscreen_app=$!
sleep 3
snap fullscreen

kill $fullscreen_app
fullscreen_app=
sleep 2
snap restored

write_config left.mp4
sleep 3
snap mirrored

# A normal window, so the workspace has something to slide away.
gjs -c '
imports.gi.versions.Gtk = "4.0";
const {Gtk, GLib} = imports.gi;
Gtk.init();
new Gtk.Window({title: "normal-test", default_width: 500, default_height: 300}).present();
new GLib.MainLoop(null, false).run();
' &
normal_app=$!
sleep 2
touch "$RWG_PROBE_DIR/request-workspace-1"
sleep 3
touch "$RWG_PROBE_DIR/request-workspace-0"
sleep 3
kill $normal_app
touch "$RWG_PROBE_DIR/request-overview"
sleep 3
# Disabling must remove every window and clone; re-enabling must bring them back.
gnome-extensions disable "$uuid"
sleep 2
snap disabled
gnome-extensions enable "$uuid"
sleep 4
snap reenabled

for name in workspace-1-mid workspace-1 workspace-0-mid workspace-0 overview; do
    [ -s "$RWG_PROBE_DIR/$name.png" ] && echo "captured $name" || echo "missing $name" >&2
done

# Every wallpaper window must cover its whole monitor once things have settled.
python3 - "$RWG_PROBE_DIR" <<'PY'
import json, pathlib, sys
failed = False
for path in sorted(pathlib.Path(sys.argv[1]).glob("*.json")):
    if path.stem.endswith("-mid"):
        continue
    state = json.loads(path.read_text())
    for window in state["windows"]:
        if not window["title"].startswith("rusty-wallpaper@"):
            continue
        m = state["monitors"][window["monitor"]]
        expected = [m["x"], m["y"], m["width"], m["height"]]
        if window["rect"] != expected:
            print(f"FAIL {path.stem}: {window['title']} is {window['rect']}, monitor is {expected}")
            failed = True
print("FAIL: geometry" if failed else "PASS: every wallpaper window covers its monitor")

def load(name):
    return json.loads((pathlib.Path(sys.argv[1]) / f"{name}.json").read_text())

def wallpaper_windows(state):
    return [w for w in state["windows"] if w["title"].startswith("rusty-wallpaper@")]

disabled = load("disabled")
if wallpaper_windows(disabled) or any(b["videoClone"] for b in disabled["overviewBackgrounds"]):
    print("FAIL: windows or clones left behind after disabling the extension")
    failed = True
elif len(wallpaper_windows(load("reenabled"))) != len(disabled["monitors"]):
    print("FAIL: re-enabling did not bring the wallpaper windows back")
    failed = True
else:
    print("PASS: disable cleans up, re-enable restores")

overview = load("overview")
backgrounds = overview["overviewBackgrounds"]
monitors_with_video = {b["monitor"] for b in backgrounds if b["videoClone"]}
if overview["overviewVisible"] and backgrounds and all(b["videoClone"] for b in backgrounds) \
        and len(monitors_with_video) == len(overview["monitors"]):
    print(f"PASS: overview shows the video on {len(backgrounds)} workspace backgrounds")
else:
    print(f"FAIL: overview backgrounds {backgrounds}")
    failed = True
sys.exit(failed)
PY
