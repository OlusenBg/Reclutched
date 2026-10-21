#!/usr/bin/env bash
# Adds Reclutched to your app launcher (menu entry + icon) for the current user.
# Run it from the repository after `cargo install --path crates/reclutched-ui --locked`.
# Undo it with: packaging/install-desktop.sh --remove
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
desktop="$data/applications/dev.reclutched.Reclutched.desktop"
icon="$data/icons/hicolor/scalable/apps/dev.reclutched.Reclutched.svg"

if [ "${1:-}" = "--remove" ]; then
    rm -f "$desktop" "$icon"
    echo "Removed the launcher entry."
    exit 0
fi

# Launchers often do not have ~/.cargo/bin in their PATH, so point at the real binary.
bin="$(command -v reclutched-ui || true)"
[ -n "$bin" ] || bin="$HOME/.cargo/bin/reclutched-ui"
if [ ! -x "$bin" ]; then
    echo "reclutched-ui is not installed yet. Run:" >&2
    echo "  cargo install --path crates/reclutched-ui --locked" >&2
    exit 1
fi

mkdir -p "$(dirname "$desktop")" "$(dirname "$icon")"
cp "$here/branding/logo/icon.svg" "$icon"
sed "s|^Exec=.*|Exec=$bin|" "$here/packaging/dev.reclutched.Reclutched.desktop" > "$desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
echo "Done. Search for \"Reclutched\" in your launcher."
