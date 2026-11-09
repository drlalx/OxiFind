#!/usr/bin/env bash
set -euo pipefail

MODE="system"
for arg in "$@"; do
    case "$arg" in
        --user)
            MODE="user"
            ;;
        --system)
            MODE="system"
            ;;
        -h|--help)
            echo "Usage: $0 [--system | --user]"
            echo ""
            echo "Options:"
            echo "  --system   Uninstall system-wide (requires sudo / root, default)"
            echo "  --user     Uninstall from current user's ~/.local"
            echo "  -h, --help Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown argument: $arg"
            echo "Run '$0 --help' for usage."
            exit 1
            ;;
    esac
done

if [[ "$MODE" == "system" && "$EUID" -ne 0 ]]; then
    echo "==> Error: System-wide uninstallation requires root privileges."
    echo "    Please run with sudo:  sudo $0"
    echo "    Or uninstall for current user:  $0 --user"
    exit 1
fi

if [[ "$MODE" == "system" ]]; then
    BIN="/usr/local/bin/oxifind"
    ICON="/usr/share/icons/hicolor/256x256/apps/oxifind.png"
    PIXMAP="/usr/share/pixmaps/oxifind.png"
    APP="/usr/share/applications/oxifind.desktop"
    APP_DIR="/usr/share/applications"
    ICON_BASE="/usr/share/icons/hicolor"
    echo "==> Uninstalling OxiFind (system-wide)..."
else
    BIN="${XDG_BIN_HOME:-$HOME/.local/bin}/oxifind"
    ICON="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps/oxifind.png"
    PIXMAP=""
    APP="${XDG_DATA_HOME:-$HOME/.local/share}/applications/oxifind.desktop"
    APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    ICON_BASE="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
    echo "==> Uninstalling OxiFind (user: ${HOME})..."
fi

if [[ -x "$BIN" ]]; then
    "$BIN" service uninstall >/dev/null 2>&1 || true
fi

rm -f "$BIN"
rm -f "$ICON"
[[ -n "$PIXMAP" ]] && rm -f "$PIXMAP"
rm -f "$APP"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$ICON_BASE" >/dev/null 2>&1 || true
fi

echo "==> OxiFind has been uninstalled."
