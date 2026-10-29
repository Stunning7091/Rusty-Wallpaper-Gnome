use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib};

use crate::config::{Config, Fit, Wallpaper};
use crate::video::Video;

/// Window titles carry the connector so the shell extension knows which
/// monitor each window belongs to.
pub const TITLE_PREFIX: &str = "rusty-wallpaper@";

pub struct Options {
    pub config_path: PathBuf,
    /// Plain decorated windows, for trying things out without the extension.
    pub windowed: bool,
}

/// Keeps one wallpaper window per monitor in sync with the config and the
/// connected monitors.
pub struct Desktop {
    app: gtk::Application,
    options: Options,
    config: Config,
    videos: HashMap<PathBuf, Video>,
    outputs: HashMap<String, Output>,
    /// Connectors currently covered by a fullscreen window.
    fullscreen: HashSet<String>,
    _config_monitor: Option<gio::FileMonitor>,
}

struct Output {
    window: gtk::Window,
    wallpaper: Wallpaper,
}

impl Desktop {
    pub fn start(app: &gtk::Application, options: Options) -> Rc<RefCell<Self>> {
        let desktop = Rc::new(RefCell::new(Self {
            app: app.clone(),
            config: load_config(&options.config_path),
            options,
            videos: HashMap::new(),
            outputs: HashMap::new(),
            fullscreen: HashSet::new(),
            _config_monitor: None,
        }));

        let display = gdk::Display::default().expect("GTK has a display");
        let weak = Rc::downgrade(&desktop);
        display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(desktop) = weak.upgrade() {
                desktop.borrow_mut().sync();
            }
        });

        let config_monitor = watch_config(&desktop);
        desktop.borrow_mut()._config_monitor = config_monitor;
        desktop.borrow_mut().sync();
        desktop
    }

    /// Handles one line of the control protocol spoken by the shell extension.
    pub fn command(&mut self, line: &str) {
        match line.split_whitespace().collect::<Vec<_>>()[..] {
            ["fullscreen", connector, state] => {
                if state == "1" {
                    self.fullscreen.insert(connector.to_owned());
                } else {
                    self.fullscreen.remove(connector);
                }
                self.update_playback();
            }
            _ => eprintln!("rusty-wallpaper: unknown command: {line}"),
        }
    }

    fn reload(&mut self) {
        self.config = load_config(&self.options.config_path);
        self.sync();
    }

    fn sync(&mut self) {
        let display = gdk::Display::default().expect("GTK has a display");
        let mut wanted = HashMap::new();
        for monitor in display.monitors().iter::<gdk::Monitor>().flatten() {
            let Some(connector) = monitor.connector() else {
                continue;
            };
            if let Some(wallpaper) = self.config.wallpaper_for(&connector) {
                wanted.insert(connector.to_string(), (monitor, wallpaper.clone()));
            }
        }

        self.outputs.retain(|connector, output| {
            let keep = wanted.get(connector).is_some_and(|(_, wallpaper)| {
                wallpaper.file == output.wallpaper.file && wallpaper.fit == output.wallpaper.fit
            });
            if !keep {
                output.window.destroy();
            }
            keep
        });

        for (connector, (monitor, wallpaper)) in wanted {
            if self.outputs.contains_key(&connector) {
                continue;
            }
            let Some(paintable) = self.paintable_for(&wallpaper.file) else {
                continue;
            };
            let window = build_window(
                &self.app,
                &connector,
                &monitor,
                &paintable,
                wallpaper.fit,
                self.options.windowed,
            );
            self.outputs.insert(connector, Output { window, wallpaper });
        }

        // Stop decoding files no monitor shows any more.
        let in_use: HashSet<&Path> = self
            .outputs
            .values()
            .map(|o| o.wallpaper.file.as_path())
            .collect();
        self.videos
            .retain(|file, _| in_use.contains(file.as_path()));
        self.update_playback();
    }

    /// Returns the frames of `file`, starting a decoder unless one already runs.
    fn paintable_for(&mut self, file: &Path) -> Option<gdk::Paintable> {
        if !self.videos.contains_key(file) {
            match Video::new(file) {
                Ok(video) => {
                    self.videos.insert(file.to_path_buf(), video);
                }
                Err(e) => {
                    eprintln!("rusty-wallpaper: {e}");
                    return None;
                }
            }
        }
        self.videos.get(file).map(|video| video.paintable().clone())
    }

    /// A shared video keeps playing as long as any monitor showing it is visible.
    fn update_playback(&self) {
        for (file, video) in &self.videos {
            let visible = self.outputs.iter().any(|(connector, output)| {
                output.wallpaper.file == *file
                    && !(self.config.pause.fullscreen && self.fullscreen.contains(connector))
            });
            video.set_playing(visible);
        }
    }
}

fn load_config(path: &Path) -> Config {
    match Config::load(path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("rusty-wallpaper: no usable config: {e}");
            Config::default()
        }
    }
}

/// Reloads the config whenever the file is saved, created or removed.
fn watch_config(desktop: &Rc<RefCell<Desktop>>) -> Option<gio::FileMonitor> {
    let file = gio::File::for_path(&desktop.borrow().options.config_path);
    let monitor = file
        .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .inspect_err(|e| eprintln!("rusty-wallpaper: cannot watch config: {e}"))
        .ok()?;
    let weak = Rc::downgrade(desktop);
    monitor.connect_changed(move |_, _, _, event| {
        use gio::FileMonitorEvent::*;
        if matches!(
            event,
            ChangesDoneHint | Created | Deleted | Renamed | MovedIn
        ) && let Some(desktop) = weak.upgrade()
        {
            desktop.borrow_mut().reload();
        }
    });
    Some(monitor)
}

fn build_window(
    app: &gtk::Application,
    connector: &str,
    monitor: &gdk::Monitor,
    paintable: &gdk::Paintable,
    fit: Fit,
    windowed: bool,
) -> gtk::Window {
    let picture = gtk::Picture::for_paintable(paintable);
    picture.set_content_fit(match fit {
        Fit::Cover => gtk::ContentFit::Cover,
        Fit::Contain => gtk::ContentFit::Contain,
        Fit::Fill => gtk::ContentFit::Fill,
    });

    let window = gtk::Window::builder()
        .application(app)
        .title(format!("{TITLE_PREFIX}{connector}"))
        .css_classes(["rusty-wallpaper"])
        .child(&picture)
        .build();

    if windowed {
        window.set_default_size(960, 540);
    } else {
        let geometry = monitor.geometry();
        window.set_decorated(false);
        window.set_default_size(geometry.width(), geometry.height());
        // Clicks pass straight through to the desktop below.
        window.connect_realize(|window| {
            if let Some(surface) = window.surface() {
                let clear_input = |surface: &gdk::Surface| {
                    surface.set_input_region(Some(&cairo::Region::create()));
                };
                clear_input(&surface);
                // GTK recomputes the input region on resize, so keep clearing it.
                surface.connect_layout(move |surface, _, _| clear_input(surface));
            }
        });
    }

    window.present();
    window
}

pub fn print_monitors() {
    let display = gdk::Display::default().expect("GTK has a display");
    for monitor in display.monitors().iter::<gdk::Monitor>().flatten() {
        let geometry = monitor.geometry();
        println!(
            "{}\t{}x{}\t{} {}",
            monitor.connector().as_deref().unwrap_or("?"),
            geometry.width(),
            geometry.height(),
            monitor.manufacturer().as_deref().unwrap_or(""),
            monitor.model().as_deref().unwrap_or(""),
        );
    }
}

pub fn install_style() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string("window.rusty-wallpaper { background: black; }");
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("GTK has a display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Feeds stdin lines from the extension into `Desktop::command` and quits
/// when the extension closes the pipe.
pub fn listen_for_commands(desktop: Rc<RefCell<Desktop>>) {
    let (sender, receiver) = async_channel::unbounded::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            let Ok(line) = line else { break };
            if sender.send_blocking(line).is_err() {
                return;
            }
        }
    });
    glib::spawn_future_local(async move {
        while let Ok(line) = receiver.recv().await {
            desktop.borrow_mut().command(&line);
        }
        let app = desktop.borrow().app.clone();
        app.quit();
    });
}
