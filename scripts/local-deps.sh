#!/usr/bin/env bash
# Ubuntu 24.04 development fallback, without sudo or changing system packages.
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p "$project_dir/.deps/debs" "$project_dir/.deps/root"
cd "$project_dir/.deps/debs"
apt-get download libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libunwind-dev libdw-dev libelf-dev liborc-0.4-dev
for package in ./*.deb; do dpkg-deb -x "$package" "$project_dir/.deps/root"; done
python3 - "$project_dir/.deps/root" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1]).resolve()
lib = root / 'usr/lib/x86_64-linux-gnu'
for pc in (lib / 'pkgconfig').glob('*.pc'):
    pc.write_text(pc.read_text().replace('prefix=/usr', f'prefix={root}/usr'))
for link in lib.glob('*.so'):
    if link.is_symlink():
        runtime = Path('/usr/lib/x86_64-linux-gnu') / link.readlink().name
        if runtime.exists():
            link.unlink()
            link.symlink_to(runtime)
PY
echo 'Dependências disponíveis em .deps/. Use scripts/dev.sh para compilar e testar.'
