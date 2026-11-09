#!/usr/bin/env bash
set -euo pipefail

MODE="user"
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
            echo "  --system   Uninstall from /Applications and /usr/local/bin (requires sudo)"
            echo "  --user     Uninstall from ~/Applications and ~/.local/bin (default)"
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
    BUNDLE="/Applications/OxiFind.app"
    CLI="/usr/local/bin/oxifind"
    echo "==> Uninstalling OxiFind (system-wide)..."
else
    BUNDLE="$HOME/Applications/OxiFind.app"
    CLI="${XDG_BIN_HOME:-$HOME/.local/bin}/oxifind"
    echo "==> Uninstalling OxiFind (user: ${HOME})..."
fi

rm -rf "$BUNDLE"
rm -f "$CLI"

echo "==> OxiFind has been uninstalled."
