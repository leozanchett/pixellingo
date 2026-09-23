#!/usr/bin/env bash
set -euo pipefail
[[ "${AREA_TRANSLATOR_ISOLATED_TEST:-}" == 1 ]] || { echo 'Requires an isolated D-Bus session.' >&2; exit 1; }
gjs -m ui/refresh.js
owner=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.areatranslator.Service)
test "$owner" = '(false,)'
"${AREA_TRANSLATOR_BINARY:-target/debug/area-translator}" >.deps/service-smoke.log 2>&1 &
service_pid=$!
trap 'kill -INT "$service_pid" 2>/dev/null || true' EXIT
python3 - <<'PY'
import ast, json, subprocess, time
base = ['gdbus', 'call', '--session', '--dest', 'io.github.areatranslator.Service',
        '--object-path', '/io/github/areatranslator/Service', '--method']
def call(method, *args):
    return subprocess.run(base + ['io.github.areatranslator.Service.' + method, *args], capture_output=True, text=True)
result = None
for attempt in range(30):
    owner = subprocess.check_output(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus',
        '--object-path', '/org/freedesktop/DBus', '--method', 'org.freedesktop.DBus.NameHasOwner',
        'io.github.areatranslator.Service'], text=True)
    if 'true' not in owner:
        time.sleep(.1)
        continue
    result = call('GetStatus')
    if result.returncode == 0:
        break
    time.sleep(.1)
assert result is not None and result.returncode == 0, 'Test service did not start'
state = json.loads(ast.literal_eval(result.stdout)[0])
assert state['state'] == 'idle', state
assert state['mode'] == 'manual' and state['manual_requests'] == 0
assert state['ocr_provider'] == 'google_cloud_vision'
assert state['subtitle_duration_seconds'] == 15
assert state['ocr_successes'] == 0 and not state['ocr_pending']
assert call('Refresh').returncode != 0, 'Refresh requires an active region'
assert call('GetCropPreview').returncode != 0, 'Crop preview requires an active capture'
assert call('BeginSelection').returncode != 0, 'Capture must require configuration'
assert call('BeginWindowSelection').returncode != 0, 'Window capture must also require configuration'
assert call('SetApiKey', 'bad key').returncode != 0
assert call('SetApiKey', 'test-only-no-network').returncode == 0
assert call('Resume').returncode != 0, 'No region selected'
assert call('SetRegion', '{"x":0,"y":0,"width":16,"height":16}', '{"x":0,"y":0,"width":1920,"height":1080}').returncode != 0
assert call('Stop').returncode == 0
assert call('Stop').returncode == 0
state_text = call('GetStatus').stdout
assert 'test-only-no-network' not in state_text
state = json.loads(ast.literal_eval(state_text)[0])
assert state['api_count'] == 0 and state['ocr_count'] == 0
assert state['manual_requests'] == 0
assert state['captured_frames'] == 0 and state['last_frame_age_ms'] is None
assert state['ocr_text'] == '' and state['ocr_confidence'] is None
assert state['api_successes'] == 0 and not state['api_pending']
assert state['source_type'] is None
print('D-Bus lifecycle, validation, idempotent stop and credential redaction passed.')
PY
kill -INT "$service_pid"
wait "$service_pid"
trap - EXIT
