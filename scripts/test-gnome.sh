#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"
test_extension="$project_dir/.deps/gnome-data/gnome-shell/extensions/area-translator@local"
mkdir -p "$test_extension/schemas" .deps/gnome-config .deps/gnome-cache .deps/gnome-run
chmod 700 .deps/gnome-run
cp extension/*.js extension/*.json extension/*.css "$test_extension/"
cat tests/gnome-instrumentation.txt >> "$test_extension/extension.js"
cp extension/schemas/*.xml "$test_extension/schemas/"
glib-compile-schemas --strict "$test_extension/schemas"
timeout 50s dbus-run-session -- env AREA_TRANSLATOR_ISOLATED_TEST=1 \
    XDG_DATA_HOME="$project_dir/.deps/gnome-data" XDG_CONFIG_HOME="$project_dir/.deps/gnome-config" \
    XDG_CACHE_HOME="$project_dir/.deps/gnome-cache" XDG_RUNTIME_DIR="$project_dir/.deps/gnome-run" \
    GSETTINGS_BACKEND=memory GIO_USE_VFS=local GNOME_SHELL_SESSION_MODE=user \
    XDG_CURRENT_DESKTOP=GNOME WAYLAND_DISPLAY=area-translator-test GDK_BACKEND=wayland \
    GDK_DEBUG=no-portals bash tests/gnome-smoke.sh
