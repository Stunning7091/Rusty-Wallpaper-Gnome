// SPDX-License-Identifier: MIT
//
// Test-only extension used by scripts/test-headless.sh. When a file named
// request-<name> appears in $RWG_PROBE_DIR it writes <name>.json (monitor and
// window state) and <name>.png (a screenshot of the whole stage).

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const typeName = type => Object.keys(Meta.WindowType).find(key => Meta.WindowType[key] === type);

export default class ProbeExtension extends Extension {
    enable() {
        this._dir = GLib.getenv('RWG_PROBE_DIR');
        this._timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
            this._tick();
            return GLib.SOURCE_CONTINUE;
        });
    }

    disable() {
        GLib.source_remove(this._timeoutId);
    }

    _tick() {
        // GNOME opens the overview at login; the wallpaper is only visible outside it.
        if (Main.overview.visible && !Main.overview.animationInProgress)
            Main.overview.hide();
        if (!this._dir)
            return;

        const enumerator = Gio.File.new_for_path(this._dir).enumerate_children('standard::name', 0, null);
        for (const info of enumerator) {
            const fileName = info.get_name();
            if (!fileName.startsWith('request-'))
                continue;
            GLib.unlink(GLib.build_filenamev([this._dir, fileName]));
            this._capture(fileName.slice('request-'.length));
        }
    }

    _capture(name) {
        const state = {
            overviewVisible: Main.overview.visible,
            monitors: Main.layoutManager.monitors.map(m => ({
                index: m.index, x: m.x, y: m.y, width: m.width, height: m.height, inFullscreen: m.inFullscreen,
            })),
            focus: global.display.focus_window?.get_title() ?? null,
            tabList: global.display.get_tab_list(Meta.TabList.NORMAL, null).map(w => w.get_title()),
            windows: global.get_window_actors().map(actor => {
                const window = actor.meta_window;
                const rect = window.get_frame_rect();
                return {
                    title: window.get_title(),
                    type: typeName(window.get_window_type()),
                    monitor: window.get_monitor(),
                    rect: [rect.x, rect.y, rect.width, rect.height],
                    onAllWorkspaces: window.is_on_all_workspaces(),
                    skipTaskbar: window.is_skip_taskbar(),
                    fullscreen: window.is_fullscreen(),
                };
            }),
        };
        GLib.file_set_contents(GLib.build_filenamev([this._dir, `${name}.json`]), JSON.stringify(state, null, 2));

        const stream = Gio.File.new_for_path(GLib.build_filenamev([this._dir, `${name}.png`]))
            .replace(null, false, Gio.FileCreateFlags.NONE, null);
        new Shell.Screenshot().screenshot(false, stream, (screenshot, result) => {
            try {
                screenshot.screenshot_finish(result);
            } catch (e) {
                console.error(`probe: screenshot failed: ${e.message}`);
            }
            stream.close(null);
        });
    }
}
