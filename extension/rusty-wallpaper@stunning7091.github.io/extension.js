// SPDX-License-Identifier: MIT
//
// Starts the rusty-wallpaper player as a privileged Wayland client and turns
// its windows into desktop windows: below everything, on every workspace, out
// of Alt+Tab and the overview, and never focused. The overview draws its own
// static backgrounds, so it gets clones of those windows on top of them.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {WorkspaceBackground} from 'resource:///org/gnome/shell/ui/workspace.js';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';

// Must match TITLE_PREFIX in src/desktop.rs.
const TITLE_PREFIX = 'rusty-wallpaper@';
const PLAYER_NAME = 'rusty-wallpaper';

const RESTART_DELAY_MS = 2000;
// Give up after this many crashes in a row that happen soon after starting.
const MAX_QUICK_CRASHES = 5;
const QUICK_CRASH_US = 10 * GLib.USEC_PER_SEC;

// Fills the space the overview gives the background but never asks for any:
// a plain clone requests the full window size and makes the overview lay the
// workspace out wider than GNOME intends, stretching it.
const BackgroundClone = GObject.registerClass(
class BackgroundClone extends Clutter.Clone {
    vfunc_get_preferred_width(_forHeight) {
        return [0, 0];
    }

    vfunc_get_preferred_height(_forWidth) {
        return [0, 0];
    }
});

export default class RustyWallpaperExtension extends Extension {
    enable() {
        // Meta.Window -> {connector, monitorIndex, signalIds}
        this._windows = new Map();
        // overview WorkspaceBackground -> Clutter.Clone of that monitor's player window
        this._overviewClones = new Map();
        // connector -> last fullscreen state sent to the player
        this._fullscreen = new Map();
        this._quickCrashes = 0;
        this._restartId = 0;

        this._displaySignalIds = [
            global.display.connect('window-created', (_display, window) => this._onWindowCreated(window)),
            global.display.connect('in-fullscreen-changed', () => this._syncFullscreen()),
            global.display.connect('notify::focus-window', () => this._refuseFocus()),
        ];
        this._monitorsChangedId = Main.layoutManager.connect('monitors-changed', () => this._placeAll());

        this._injectionManager = new InjectionManager();
        const extension = this;
        this._injectionManager.overrideMethod(WorkspaceBackground.prototype, '_init', originalInit => {
            return function (...args) {
                originalInit.call(this, ...args);
                extension._addOverviewClone(this);
            };
        });
        // Some workspace backgrounds already exist before the extension is enabled.
        for (const background of findWorkspaceBackgrounds(Main.layoutManager.overviewGroup))
            this._addOverviewClone(background);

        this._launch();
    }

    disable() {
        this._injectionManager.clear();
        for (const [background, clone] of this._overviewClones) {
            background.disconnectObject(this);
            clone.destroy();
        }
        this._overviewClones = null;

        for (const id of this._displaySignalIds)
            global.display.disconnect(id);
        Main.layoutManager.disconnect(this._monitorsChangedId);
        for (const [window, {signalIds}] of this._windows) {
            for (const id of signalIds)
                window.disconnect(id);
        }
        if (this._restartId)
            GLib.source_remove(this._restartId);

        // Clearing _process first tells the exit handler this was intentional.
        const process = this._process;
        this._process = null;
        process?.force_exit();

        this._client = null;
        this._stdin = null;
        this._windows = null;
        this._fullscreen = null;
    }

    _findPlayer() {
        const candidates = [
            GLib.getenv('RUSTY_WALLPAPER_BIN'),
            GLib.build_filenamev([GLib.get_home_dir(), '.local', 'bin', PLAYER_NAME]),
            GLib.find_program_in_path(PLAYER_NAME),
        ];
        return candidates.find(path => path && GLib.file_test(path, GLib.FileTest.IS_EXECUTABLE)) ?? null;
    }

    _launch() {
        this._restartId = 0;
        const player = this._findPlayer();
        if (!player) {
            console.error(`${this.uuid}: ${PLAYER_NAME} not found in ~/.local/bin or PATH`);
            return;
        }

        const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.STDIN_PIPE});
        // The player has to connect through the socket mutter hands it, not via Xwayland.
        launcher.setenv('GDK_BACKEND', 'wayland', true);
        try {
            this._client = Meta.WaylandClient.new(global.context, launcher);
            this._process = this._client.spawnv(global.display, [player, '--managed']);
        } catch (e) {
            console.error(`${this.uuid}: cannot start ${player}: ${e.message}`);
            this._client = null;
            return;
        }
        this._stdin = this._process.get_stdin_pipe();
        this._fullscreen.clear();
        this._startedAt = GLib.get_monotonic_time();

        const process = this._process;
        process.wait_async(null, (_process, result) => {
            try {
                process.wait_finish(result);
            } catch {}
            if (process === this._process)
                this._onPlayerExited(process);
        });
    }

    _onPlayerExited(process) {
        this._process = null;
        this._client = null;
        this._stdin = null;
        this._windows.clear();
        this._updateOverviewClones();

        const status = process.get_if_exited()
            ? `exit status ${process.get_exit_status()}`
            : `signal ${process.get_term_sig()}`;
        if (GLib.get_monotonic_time() - this._startedAt < QUICK_CRASH_US)
            this._quickCrashes++;
        else
            this._quickCrashes = 0;

        if (this._quickCrashes >= MAX_QUICK_CRASHES) {
            console.error(`${this.uuid}: player keeps stopping (${status}); giving up until the extension is re-enabled`);
            return;
        }
        console.warn(`${this.uuid}: player stopped (${status}); restarting`);
        this._restartId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, RESTART_DELAY_MS, () => {
            this._launch();
            return GLib.SOURCE_REMOVE;
        });
    }

    _onWindowCreated(window) {
        if (!this._client?.owns_window(window))
            return;

        this._client.make_desktop(window);
        const record = {connector: null, monitorIndex: -1, signalIds: []};
        record.signalIds = [
            // On Wayland the title can arrive after the window is created.
            window.connect('notify::title', () => this._place(window)),
            window.connect('shown', () => this._place(window)),
            window.connect('unmanaged', () => this._forgetWindow(window)),
        ];
        this._windows.set(window, record);
        this._place(window);
    }

    _place(window) {
        const record = this._windows?.get(window);
        const title = window.get_title() ?? '';
        if (!record || !title.startsWith(TITLE_PREFIX))
            return;

        record.connector = title.slice(TITLE_PREFIX.length);
        const index = global.backend.get_monitor_manager().get_monitor_for_connector(record.connector);
        record.monitorIndex = index;
        this._updateOverviewClones();
        if (index < 0)
            return;

        const geometry = global.display.get_monitor_geometry(index);
        window.stick();
        window.move_to_monitor(index);
        window.move_resize_frame(false, geometry.x, geometry.y, geometry.width, geometry.height);
        this._syncFullscreen();
    }

    _placeAll() {
        for (const window of this._windows.keys())
            this._place(window);
    }

    _forgetWindow(window) {
        if (!this._windows?.delete(window))
            return;
        this._updateOverviewClones();
    }

    _addOverviewClone(background) {
        // Above GNOME's static background, below the window previews.
        const clone = new BackgroundClone({x_expand: true, y_expand: true});
        background._bin.add_child(clone);
        this._overviewClones.set(background, clone);
        background.connectObject('destroy', () => this._overviewClones?.delete(background), this);
        clone.source = this._actorForMonitor(background._monitorIndex);
    }

    _updateOverviewClones() {
        for (const [background, clone] of this._overviewClones)
            clone.source = this._actorForMonitor(background._monitorIndex);
    }

    _actorForMonitor(index) {
        for (const [window, record] of this._windows) {
            if (record.monitorIndex === index)
                return window.get_compositor_private();
        }
        return null;
    }

    _syncFullscreen() {
        const monitorManager = global.backend.get_monitor_manager();
        for (const {connector} of this._windows.values()) {
            if (!connector)
                continue;
            const index = monitorManager.get_monitor_for_connector(connector);
            const fullscreen = index >= 0 && global.display.get_monitor_in_fullscreen(index);
            if (this._fullscreen.get(connector) === fullscreen)
                continue;
            this._fullscreen.set(connector, fullscreen);
            this._send(`fullscreen ${connector} ${fullscreen ? 1 : 0}`);
        }
    }

    _refuseFocus() {
        const window = global.display.focus_window;
        if (window && this._client?.owns_window(window))
            global.display.unset_input_focus(global.get_current_time());
    }

    _send(line) {
        try {
            this._stdin?.write_all(new TextEncoder().encode(`${line}\n`), null);
        } catch (e) {
            console.warn(`${this.uuid}: cannot talk to the player: ${e.message}`);
        }
    }
}

function findWorkspaceBackgrounds(actor) {
    if (actor instanceof WorkspaceBackground)
        return [actor];
    return actor.get_children().flatMap(findWorkspaceBackgrounds);
}
