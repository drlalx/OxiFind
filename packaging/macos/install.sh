#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

MODE="user"
WITH_SERVICE=false

if [[ $# -eq 0 ]]; then
    if [ -t 0 ]; then
        echo "=========================================================="
        echo "               OxiFind macOS Installer                   "
        echo "=========================================================="
        echo ""
        echo "Select installation scope:"
        echo "  [1] Current user (~/Applications and ~/.local/bin) [default]"
        echo "  [2] System-wide (/Applications and /usr/local/bin, requires sudo)"
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
        echo "Do you want to enable the background indexer service (launchd)?"
        echo "Runs in background with delayed start and keeps database updated."
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
                echo "  --system        Install to /Applications and /usr/local/bin (requires sudo)"
                echo "  --user          Install to ~/Applications and ~/.local/bin (default)"
                echo "  --with-service  Also install and enable the background indexer service (launchd)"
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

if [[ "$MODE" == "system" ]]; then
    APP_DIR="/Applications"
    BIN_DIR="/usr/local/bin"
    echo "==> Starting SYSTEM-WIDE installation of OxiFind on macOS..."
else
    APP_DIR="$HOME/Applications"
    BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
    echo "==> Starting USER installation of OxiFind on macOS (${HOME})..."
fi

BINARY_PATH="$REPO_ROOT/target/release/oxifind"

if [[ ! -f "$BINARY_PATH" ]]; then
    echo "==> Release binary not found at $BINARY_PATH."
    echo "==> Building OxiFind with cargo..."
    if [[ -n "${SUDO_USER:-}" && "$EUID" -eq 0 ]]; then
        su - "$SUDO_USER" -c "cd '$REPO_ROOT' && cargo build --release"
    else
        (cd "$REPO_ROOT" && cargo build --release)
    fi
fi

BUNDLE_PATH="$APP_DIR/OxiFind.app"
CONTENTS_PATH="$BUNDLE_PATH/Contents"
MACOS_PATH="$CONTENTS_PATH/MacOS"
RESOURCES_PATH="$CONTENTS_PATH/Resources"

echo "==> Creating macOS App Bundle at $BUNDLE_PATH..."
mkdir -p "$MACOS_PATH" "$RESOURCES_PATH"

# Copy binary
cp "$BINARY_PATH" "$MACOS_PATH/oxifind"
chmod +x "$MACOS_PATH/oxifind"

# Copy Info.plist
cp "$SCRIPT_DIR/Info.plist" "$CONTENTS_PATH/Info.plist"

# Copy & process icon
if [[ -f "$REPO_ROOT/resources/oxifind.png" ]]; then
    cp "$REPO_ROOT/resources/oxifind.png" "$RESOURCES_PATH/oxifind.png"
    # Convert to .icns if iconutil and sips are available (standard on macOS)
    if command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
        TEMP_DIR="$(mktemp -d)"
        ICONSET_DIR="$TEMP_DIR/oxifind.iconset"
        mkdir -p "$ICONSET_DIR"
        sips -z 16 16     "$REPO_ROOT/resources/oxifind.png" --out "$ICONSET_DIR/icon_16x16.png" >/dev/null 2>&1 || true
        sips -z 32 32     "$REPO_ROOT/resources/oxifind.png" --out "$ICONSET_DIR/icon_16x16@2x.png" >/dev/null 2>&1 || true
        sips -z 32 32     "$REPO_ROOT/resources/oxifind.png" --out "$ICONSET_DIR/icon_32x32.png" >/dev/null 2>&1 || true
        sips -z 64 64     "$REPO_ROOT/resources/oxifind.png" --out "$ICONSET_DIR/icon_32x32@2x.png" >/dev/null 2>&1 || true
        sips -z 128 128   "$REPO_ROOT/resources/oxifind.png" --out "$ICONSET_DIR/icon_128x128.png" >/dev/null 2>&1 || true
        sips -z 256 256   "$REPO_ROOT/resources/oxifind.png" --out "$ICONSET_DIR/icon_128x128@2x.png" >/dev/null 2>&1 || true
        sips -z 256 256   "$REPO_ROOT/resources/oxifind.png" --out "$ICONSET_DIR/icon_256x256.png" >/dev/null 2>&1 || true
        iconutil -c icns "$ICONSET_DIR" -o "$RESOURCES_PATH/oxifind.icns" >/dev/null 2>&1 || true
        rm -rf "$TEMP_DIR"
    fi
fi

# CLI Symlink
mkdir -p "$BIN_DIR"
ln -sf "$MACOS_PATH/oxifind" "$BIN_DIR/oxifind"

# Update launch services bundle timestamp
touch "$BUNDLE_PATH"

# Background service setup if requested
if [[ "$WITH_SERVICE" == "true" ]]; then
    echo "==> Setting up background service..."
    "$BIN_DIR/oxifind" service install
fi

echo "==> Installation complete!"
echo "    App Bundle: $BUNDLE_PATH"
echo "    CLI Link:   $BIN_DIR/oxifind"
if [[ "$WITH_SERVICE" == "true" ]]; then
    echo "    Service: launchd io.github.oxifind (enabled & active)"
fi
echo ""
echo "You can now open OxiFind from Spotlight / Launchpad or run 'oxifind' in your terminal."
