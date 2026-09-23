#!/usr/bin/env bash
# Run INSIDE an isolated dbus-run-session, never against the user's GNOME Shell.
set -euo pipefail
[[ "${AREA_TRANSLATOR_ISOLATED_TEST:-}" == 1 ]] || { echo 'Use scripts/test-gnome.sh' >&2; exit 1; }
dbus-update-activation-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR XDG_DATA_HOME XDG_CONFIG_HOME \
    XDG_CACHE_HOME XDG_CURRENT_DESKTOP GNOME_SHELL_SESSION_MODE GDK_BACKEND GDK_DEBUG GIO_USE_VFS
gnome-shell --headless --wayland --no-x11 --virtual-monitor=1280x720 \
    --wayland-display=area-translator-test >.deps/gnome-smoke.log 2>&1 &
shell_pid=$!
mock_pid=''
trap 'kill "$shell_pid" ${mock_pid:+"$mock_pid"} 2>/dev/null || true' EXIT
for attempt in $(seq 1 30); do
    if gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
        --method org.freedesktop.DBus.Peer.Ping >/dev/null 2>&1; then break; fi
    sleep 1
done
for attempt in $(seq 1 15); do
    if gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
        --method org.gnome.Shell.Extensions.EnableExtension area-translator@local >.deps/gnome-enable.txt 2>/dev/null; then break; fi
    sleep 1
done
for attempt in $(seq 1 15); do
    if gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Overlay \
        --method io.github.areatranslator.Overlay.GetMonitors >.deps/gnome-monitors.txt 2>/dev/null; then break; fi
    sleep 1
done
cat .deps/gnome-monitors.txt
test -s .deps/gnome-monitors.txt
version=$(gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Overlay --method io.github.areatranslator.Overlay.GetVersion)
test "$version" = '(uint32 2,)'
gjs -m tests/mock-service.js &
mock_pid=$!
gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Test \
    --method io.github.areatranslator.Test.HideOverview
sleep 2
gjs -m ui/refresh.js
refreshes=$(gdbus call --session --dest io.github.areatranslator.Service --object-path /io/github/areatranslator/Service --method io.github.areatranslator.Service.GetRefreshes)
test "$refreshes" = '(uint32 1,)'
gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Test \
    --method io.github.areatranslator.Test.Inspect >.deps/overlay-state.txt
python3 - <<'PY'
from pathlib import Path
import ast, json
state = json.loads(ast.literal_eval(Path('.deps/overlay-state.txt').read_text())[0])
assert state['visible'] and not state['reactive'] and not state['canFocus'], state
assert state['text'] == 'A porta está trancada. Encontre a chave.', state
assert state['y'] + state['height'] <= 492 or state['y'] >= 648, state
assert state['fullscreen'] and state['focusedTitle'] == 'Area Translator — Synthetic Game', state
print(state)
PY
gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Test \
    --method io.github.areatranslator.Test.Screenshot "$PWD/.deps/overlay-test.png"
gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Test \
    --method io.github.areatranslator.Test.ClickSubtitle
sleep 1
clicks=$(gdbus call --session --dest io.github.areatranslator.Service --object-path /io/github/areatranslator/Service --method io.github.areatranslator.Service.GetClicks)
echo "Clicks received by fullscreen app: $clicks"
gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Test \
    --method io.github.areatranslator.Test.Inspect
test "$clicks" = '(uint32 1,)'
python3 - <<'PY'
import ast, json, subprocess, time
base = ['gdbus', 'call', '--session']
def inspect():
    result = subprocess.check_output(base + ['--dest', 'org.gnome.Shell', '--object-path', '/io/github/areatranslator/Test', '--method', 'io.github.areatranslator.Test.Inspect'], text=True)
    return json.loads(ast.literal_eval(result)[0])
def subtitle(method):
    subprocess.check_call(base + ['--dest', 'io.github.areatranslator.Service', '--object-path', '/io/github/areatranslator/Service', '--method', 'io.github.areatranslator.Service.' + method], stdout=subprocess.DEVNULL)
    time.sleep(.3)
subtitle('LongSubtitle')
long = inspect()
assert long['visible'] and long['height'] > 92, long
assert long['y'] + long['height'] <= 492, long
assert not long['reactive'] and not long['canFocus'], long
subtitle('ShortSubtitle')
short = inspect()
assert short['visible'] and short['height'] < long['height'], short
print('Subtitle grows for long translations and shrinks for short ones without overlapping capture.')
subtitle('WindowSource')
window = inspect()
assert window['visible'] and window['y'] + window['height'] == 696, window
assert not window['reactive'] and not window['canFocus'], window
subtitle('LongSubtitle')
window_long = inspect()
assert window_long['visible'] and window_long['height'] > window['height'], window_long
assert window_long['y'] + window_long['height'] == 696, window_long
print('Window capture keeps the subtitle at the monitor bottom, independent of the window crop.')
PY
gdbus call --session --dest io.github.areatranslator.Service --object-path /io/github/areatranslator/Service \
    --method io.github.areatranslator.Service.Pause
sleep 1
gdbus call --session --dest org.gnome.Shell --object-path /io/github/areatranslator/Test \
    --method io.github.areatranslator.Test.Inspect >.deps/overlay-paused.txt
python3 - <<'PY'
from pathlib import Path
import ast, json
assert not json.loads(ast.literal_eval(Path('.deps/overlay-paused.txt').read_text())[0])['visible']
PY
gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.gnome.Shell.Extensions.GetExtensionInfo area-translator@local
gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.gnome.Shell.Extensions.DisableExtension area-translator@local
echo 'GNOME 46 extension enabled, exported monitors and disabled successfully.'
