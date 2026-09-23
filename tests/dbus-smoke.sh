#!/usr/bin/env bash
set -euo pipefail
[[ "${AREA_TRANSLATOR_ISOLATED_TEST:-}" == 1 ]] || { echo 'Requires an isolated D-Bus session.' >&2; exit 1; }
"${AREA_TRANSLATOR_BINARY:-target/debug/area-translator}" >.deps/service-smoke.log 2>&1 &
service_pid=$!
trap 'kill -INT "$service_pid" 2>/dev/null || true' EXIT
python3 - <<'PY'
import ast, json, subprocess, time
base = ['gdbus', 'call', '--session', '--dest', 'io.github.areatranslator.Service',
        '--object-path', '/io/github/areatranslator/Service', '--method']
def call(method, *args):
    return subprocess.run(base + ['io.github.areatranslator.Service.' + method, *args], capture_output=True, text=True)
for attempt in range(30):
    result = call('GetStatus')
    if result.returncode == 0:
        break
    time.sleep(.1)
assert result.returncode == 0, result.stderr
state = json.loads(ast.literal_eval(result.stdout)[0])
assert state['state'] == 'idle', state
assert call('BeginSelection').returncode != 0, 'Capture must require configuration'
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
print('D-Bus lifecycle, validation, idempotent stop and credential redaction passed.')
PY
kill -INT "$service_pid"
wait "$service_pid"
trap - EXIT
