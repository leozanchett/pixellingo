#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"
if [[ -d .deps/root ]]; then
    export PKG_CONFIG_PATH="$project_dir/.deps/root/usr/lib/x86_64-linux-gnu/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    export LD_LIBRARY_PATH="$project_dir/.deps/root/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export TESSDATA_PREFIX="$project_dir/.deps/root/usr/share/tesseract-ocr/5/tessdata"
    if [[ ! -f "$TESSDATA_PREFIX/eng.traineddata" ]]; then
        export TESSDATA_PREFIX="$project_dir/.deps/root/usr/share/tesseract-ocr/4.00/tessdata"
    fi
fi
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export OMP_THREAD_LIMIT=1
exec "$@"
