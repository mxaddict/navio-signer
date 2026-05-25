#!/usr/bin/env bash
# Install the navio-signer launchd agent under the current user.
#
# Substitutes __BIN__ and __HOME__ in the plist template and copies it to
# ~/Library/LaunchAgents/. Loads the service via launchctl.
#
# Usage:
#   ./contrib/install.sh [--bin /absolute/path/to/navio-signerd]
#
# If --bin is omitted, defaults to $(command -v navio-signerd).

set -euo pipefail

bin=""
while [ $# -gt 0 ]; do
    case "$1" in
        --bin)
            bin="$2"
            shift 2
            ;;
        -h|--help)
            sed -n '2,11p' "$0"
            exit 0
            ;;
        *)
            echo "unknown arg: $1" >&2
            exit 2
            ;;
    esac
done

if [ -z "$bin" ]; then
    bin="$(command -v navio-signerd || true)"
fi

if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "navio-signerd not found. Pass --bin /absolute/path or add it to PATH first." >&2
    exit 1
fi

case "$bin" in
    /*) ;;
    *)
        echo "bin must be absolute: $bin" >&2
        exit 1
        ;;
esac

template="$(cd "$(dirname "$0")" && pwd)/launchd/sh.navio.signer.plist"
if [ ! -f "$template" ]; then
    echo "missing template: $template" >&2
    exit 1
fi

target_dir="$HOME/Library/LaunchAgents"
target="$target_dir/sh.navio.signer.plist"
log_dir="$HOME/Library/Logs/navio-signer"

mkdir -p "$target_dir" "$log_dir"

sed -e "s|__BIN__|$bin|g" -e "s|__HOME__|$HOME|g" "$template" > "$target"
chmod 0644 "$target"

if launchctl list | grep -q '^[^[:space:]]\+[[:space:]]\+[^[:space:]]\+[[:space:]]\+sh\.navio\.signer$'; then
    launchctl unload "$target" 2>/dev/null || true
fi
launchctl load "$target"

echo "installed: $target"
echo "logs:      $log_dir/{stdout,stderr}.log"
echo "stop:      launchctl unload $target"
