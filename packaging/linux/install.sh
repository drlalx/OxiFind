#!/usr/bin/env bash
set -euo pipefail

# Determine repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

MODE="user"
WITH_SERVICE=false

if [[ $# -eq 0 ]]; then
    if [ -t 0 ]; then
        echo "=========================================================="
        echo "               OxiFind Linux Installer                   "
        echo "=========================================================="
        echo ""
        echo "Select installation scope:"
        echo "  [1] Current user only (~/.local - Recommended, no sudo needed)"
        echo "  [2] System-wide (/usr/local - Requires sudo / root)"
        read -r -p "Enter choice [1/2] (default: 1): " SCOPE_CHOICE
        case "${SCOPE_CHOICE:-1}" in
            2)
                MODE="system"
                ;;
            *)
                MODE="user"
                ;;
        esac

        echo ""
        echo "Do you want to enable the background indexer service?"
        echo "Runs in background (systemd --user) with delayed start and"
        echo "keeps the database (~/.local/share/oxifind/index.snapshot) up-to-date."
        read -r -p "Enable background service? [y/N] (default: N): " SERVICE_CHOICE
        case "${SERVICE_CHOICE:-n}" in
            [yY]|[yY][eE][sS])
                WITH_SERVICE=true
                ;;
            *)
                WITH_SERVICE=false
                ;;
        esac

        echo ""
        echo "Installation summary:"
        echo "  - Scope:   $MODE"
        echo "  - Service: $([[ "$WITH_SERVICE" == "true" ]] && echo "Yes (enabled)" || echo "No")"
        echo ""
        read -r -p "Proceed with installation? [Y/n] (default: Y): " CONFIRM_CHOICE
        case "${CONFIRM_CHOICE:-y}" in
            [nN]|[nN][oO])
                echo "Installation cancelled."
                exit 0
                ;;
            *)
                echo "Proceeding with installation..."
                echo ""
                ;;
        esac
    else
        MODE="user"
        WITH_SERVICE=false
    fi
else
    for arg in "$@"; do
        case "$arg" in
            --user)
                MODE="user"
                ;;
            --system)
                MODE="system"
                ;;
            --with-service|--service)
                WITH_SERVICE=true
                ;;
            -h|--help)
                echo "Usage: $0 [--system | --user] [--with-service]"
                echo ""
                echo "Options:"
                echo "  --system        Install system-wide for all users (requires sudo / root)"
                echo "  --user          Install locally for current user in ~/.local (default)"
                echo "  --with-service  Also install and enable the background indexer service (systemd)"
                echo "  -h, --help      Show this help message"
                echo ""
                echo "Running without arguments prompts interactively for installation options."
                exit 0
                ;;
            *)
                echo "Unknown argument: $arg"
                echo "Run '$0 --help' for usage."
                exit 1
                ;;
        esac
    done
fi

if [[ "$MODE" == "system" && "$EUID" -ne 0 ]]; then
    echo "==> System-wide installation requires root privileges."
    if command -v sudo >/dev/null 2>&1; then
        echo "    Re-running installer with sudo..."
        EXTRA_ARGS=""
        if [[ "$WITH_SERVICE" == "true" ]]; then
            EXTRA_ARGS="--with-service"
        fi
        exec sudo "$0" --system $EXTRA_ARGS
    else
        echo "    Error: sudo not available. Please run as root."
        exit 1
    fi
fi

# Define destination paths
if [[ "$MODE" == "system" ]]; then
    BIN_DIR="/usr/local/bin"
    ICON_DIR="/usr/share/icons/hicolor/256x256/apps"
    PIXMAP_DIR="/usr/share/pixmaps"
    APP_DIR="/usr/share/applications"
    ICON_BASE="/usr/share/icons/hicolor"
    echo "==> Starting SYSTEM-WIDE installation of OxiFind..."
else
    BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
    ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
    PIXMAP_DIR=""
    APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    ICON_BASE="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
    echo "==> Starting USER installation of OxiFind (${HOME})..."
fi

BINARY_PATH="$REPO_ROOT/target/release/oxifind"

# Check if release binary exists, build if missing
if [[ ! -f "$BINARY_PATH" ]]; then
    echo "==> Release binary not found at $BINARY_PATH."
    echo "==> Building OxiFind in release mode..."
    if [[ -n "${SUDO_USER:-}" && "$EUID" -eq 0 ]]; then
        # Run cargo build as original user to prevent root ownership of target/
        su - "$SUDO_USER" -c "cd '$REPO_ROOT' && cargo build --release"
    else
        (cd "$REPO_ROOT" && cargo build --release)
    fi
fi

# 1. Install binary
echo "==> Installing binary to $BIN_DIR/oxifind..."
install -d "$BIN_DIR"
install -m 755 "$BINARY_PATH" "$BIN_DIR/oxifind"

# 2. Install icon
echo "==> Installing icon to $ICON_DIR/oxifind.png..."
install -d "$ICON_DIR"
install -m 644 "$REPO_ROOT/resources/oxifind.png" "$ICON_DIR/oxifind.png"

if [[ -n "$PIXMAP_DIR" ]]; then
    install -d "$PIXMAP_DIR"
    install -m 644 "$REPO_ROOT/resources/oxifind.png" "$PIXMAP_DIR/oxifind.png"
fi

# 3. Install desktop launcher
echo "==> Installing desktop launcher to $APP_DIR/oxifind.desktop..."
install -d "$APP_DIR"
install -m 644 "$REPO_ROOT/packaging/linux/oxifind.desktop" "$APP_DIR/oxifind.desktop"

# 4. Refresh desktop and icon databases if available
echo "==> Updating desktop and icon caches..."
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$ICON_BASE" >/dev/null 2>&1 || true
fi

# 5. Background service setup if requested
if [[ "$WITH_SERVICE" == "true" ]]; then
    echo "==> Setting up background service..."
    if [[ "$MODE" == "user" ]]; then
        "$BIN_DIR/oxifind" service install
    elif [[ -n "${SUDO_USER:-}" ]]; then
        su - "$SUDO_USER" -c "$BIN_DIR/oxifind service install"
    fi
fi

echo "==> Installation complete!"
echo "    Binary:  $BIN_DIR/oxifind"
echo "    Desktop: $APP_DIR/oxifind.desktop"
echo "    Icon:    $ICON_DIR/oxifind.png"
if [[ "$WITH_SERVICE" == "true" ]]; then
    echo "    Service: systemd --user oxifind.service (enabled & active)"
fi
echo ""
echo "You can now run 'oxifind' from terminal or find OxiFind in your application launcher."
