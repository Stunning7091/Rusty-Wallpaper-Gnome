mod config;
mod desktop;
mod video;

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::ExitCode;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::config::Config;
use crate::desktop::{Desktop, Options};

const APP_ID: &str = "io.github.stunning7091.RustyWallpaperGnome";

const USAGE: &str = "\
Usage: rusty-wallpaper [OPTIONS]

Plays looping videos as the desktop wallpaper, one window per monitor.
On GNOME the companion shell extension starts this program and moves its
windows behind the desktop; run it directly only for testing.

Options:
  --config <PATH>  Config file (default: ~/.config/rusty-wallpaper/config.toml)
  --windowed       Open normal, decorated windows (testing without the extension)
  --managed        Read control commands from stdin and quit when it closes
                   (used by the shell extension)
  --list-monitors  Print the connector names to use in the config, then exit
  -h, --help       Show this help
  -V, --version    Show the version";

struct Args {
    options: Options,
    managed: bool,
    list_monitors: bool,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut config_path = None;
    let mut windowed = false;
    let mut managed = false;
    let mut list_monitors = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                config_path = Some(PathBuf::from(args.next().ok_or("--config needs a path")?))
            }
            "--windowed" => windowed = true,
            "--managed" => managed = true,
            "--list-monitors" => list_monitors = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("rusty-wallpaper {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(Args {
        options: Options {
            config_path: config_path.unwrap_or_else(Config::default_path),
            windowed,
        },
        managed,
        list_monitors,
    }))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rusty-wallpaper: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = gst::init() {
        eprintln!("rusty-wallpaper: cannot initialise GStreamer: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = gstgtk4::plugin_register_static() {
        eprintln!("rusty-wallpaper: cannot register the GTK video sink: {e}");
        return ExitCode::FAILURE;
    }
    // The decoder ranking only matters for the first pipeline, so the setting
    // is read here and is not affected by later config reloads.
    if Config::load(&args.options.config_path).is_ok_and(|c| c.hardware_decoding) {
        video::prefer_hardware_decoders();
    }

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let args = RefCell::new(Some(args));
    app.connect_activate(move |app| {
        let Some(Args {
            options,
            managed,
            list_monitors,
        }) = args.take()
        else {
            return;
        };
        if list_monitors {
            desktop::print_monitors();
            return;
        }
        desktop::install_style();
        // Stay alive while no monitor has a wallpaper, e.g. before the config exists.
        std::mem::forget(app.hold());
        let desktop = Desktop::start(app, options);
        if managed {
            desktop::listen_for_commands(desktop);
        } else {
            // Unmanaged runs live until the process is killed; keep the state around.
            std::mem::forget(desktop);
        }
    });

    // Our own arguments are already handled; don't let GTK parse them.
    match app.run_with_args::<&str>(&[]) {
        glib::ExitCode::SUCCESS => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}
