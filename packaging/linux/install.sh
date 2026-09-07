#!/bin/sh
# Install the extracted release for this user; never modify package-manager-owned files.
set -eu
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_dir="$HOME/.local/lib/kbmouse"
bin_dir="$HOME/.local/bin"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
# Desktop Exec quoting has additional escaping rules. Reject uncommon special characters
# instead of creating a launcher that could execute a different command.
case "$HOME$data_dir" in *'"'*|*'\'*|*'$'*|*'`'*|*'%'*) echo 'Unsupported special character in installation path' >&2; exit 1;; esac
if [ -e "$bin_dir/kbmouse" ] && [ ! -L "$bin_dir/kbmouse" ]; then
    echo "Refusing to replace unrelated file: $bin_dir/kbmouse" >&2
    exit 1
fi
mkdir -p "$install_dir" "$bin_dir" "$data_dir/applications" "$data_dir/icons"
# Rename a staged executable so an existing running copy isn't overwritten in place.
install -m 755 "$source_dir/kbmouse" "$install_dir/kbmouse.new"
mv -f "$install_dir/kbmouse.new" "$install_dir/kbmouse"
install -m 755 "$source_dir/uninstall.sh" "$install_dir/uninstall.sh"
install -m 644 "$source_dir/logo.png" "$data_dir/icons/kbmouse.png"
ln -sfn "$install_dir/kbmouse" "$bin_dir/kbmouse"
printf '[Desktop Entry]\nType=Application\nName=kbmouse\nComment=Keyboard-driven mouse\nExec="%s"\nIcon=%s\nTerminal=false\nCategories=Utility;Accessibility;\n' \
    "$install_dir/kbmouse" "$data_dir/icons/kbmouse.png" > "$data_dir/applications/kbmouse.desktop"
printf 'Installed kbmouse. Launch it from your application menu or %s\nUninstall: %s\n' "$bin_dir/kbmouse" "$install_dir/uninstall.sh"
