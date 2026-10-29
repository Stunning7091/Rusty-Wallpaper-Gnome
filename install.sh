#!/bin/sh
# Builds the player and installs it plus the GNOME Shell extension for the current user.
set -eu

UUID=rusty-wallpaper@stunning7091.github.io
cd "$(dirname "$0")"

cargo build --release
install -Dm755 target/release/rusty-wallpaper "$HOME/.local/bin/rusty-wallpaper"

ext_dir="$HOME/.local/share/gnome-shell/extensions/$UUID"
mkdir -p "$ext_dir"
cp extension/$UUID/* "$ext_dir/"

config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/rusty-wallpaper"
if [ ! -e "$config_dir/config.toml" ]; then
    install -Dm644 config.example.toml "$config_dir/config.toml"
    echo "Created $config_dir/config.toml; point it at your videos."
fi

if gnome-extensions info "$UUID" >/dev/null 2>&1; then
    # Already known to the shell: restart it so the new player is used.
    gnome-extensions disable "$UUID" && gnome-extensions enable "$UUID"
    echo "Updated and restarted."
else
    echo "Installed. Log out and back in once, then run:"
    echo "  gnome-extensions enable $UUID"
fi
