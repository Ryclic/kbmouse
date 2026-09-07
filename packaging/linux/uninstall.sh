#!/bin/sh
set -eu
install_dir="$HOME/.local/lib/kbmouse"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
# Only remove the launcher if it still points to this installation.
if [ -L "$HOME/.local/bin/kbmouse" ] && [ "$(readlink "$HOME/.local/bin/kbmouse")" = "$install_dir/kbmouse" ]; then
    rm -- "$HOME/.local/bin/kbmouse"
fi
rm -f -- "$data_dir/applications/kbmouse.desktop" "$data_dir/icons/kbmouse.png" \
    "$install_dir/kbmouse" "$install_dir/uninstall.sh"
rmdir -- "$install_dir" 2>/dev/null || true
printf 'Uninstalled kbmouse. Your configuration has been preserved.\n'
