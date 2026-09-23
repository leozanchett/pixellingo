#!/usr/bin/env bash
set -euo pipefail
export GDK_BACKEND=x11 GTK_A11Y=none GIO_USE_VFS=local
export AREA_TRANSLATOR_TEST_IMAGE="$PWD/.deps/selection-fixture.png"
python3 - <<'PY'
from PIL import Image, ImageDraw, ImageFont
im = Image.new('RGB', (1280, 720), '#10232e')
d = ImageDraw.Draw(im)
d.rectangle((120, 500, 1160, 640), fill='#050a0f')
d.text((145, 540), 'The door is locked. Find the key.', font=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', 28), fill='white')
im.save('.deps/selection-fixture.png')
PY
gjs -m ui/app.js --smoke-selection >.deps/ui-smoke.log 2>&1 &
ui_pid=$!
trap 'kill "$ui_pid" 2>/dev/null || true' EXIT
sleep 2
python3 - <<'PY'
from PIL import ImageGrab
import os
ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save('.deps/ui-selection.png')
PY
wait "$ui_pid"
trap - EXIT
if rg 'JS ERROR|CRITICAL' .deps/ui-smoke.log; then exit 1; fi
cat .deps/ui-smoke.log
