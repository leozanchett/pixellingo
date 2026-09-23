#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"
if [[ "${1:-}" == '--local-deps' ]]; then bash scripts/local-deps.sh; fi
for tool in cargo gjs glib-compile-schemas; do
    command -v "$tool" >/dev/null || { echo "Dependência ausente: $tool" >&2; exit 1; }
done
bash scripts/dev.sh cargo build --release --locked
bash scripts/dev.sh target/release/area-translator --check
install_prefix="${AREA_TRANSLATOR_PREFIX:-$HOME/.local}"
install_root="$install_prefix/share/area-translator"
bin_dir="$install_prefix/bin"
extension_root="$install_prefix/share/gnome-shell/extensions/area-translator@local"
mkdir -p "$install_root/bin" "$install_root/ui" "$install_root/lib" "$install_root/tessdata" \
    "$bin_dir" "$extension_root/schemas" "$install_prefix/share/applications" "$install_prefix/share/dbus-1/services"
install -m755 target/release/area-translator "$install_root/bin/area-translator"
install -m644 ui/app.js "$install_root/ui/app.js"
install -m644 extension/*.js extension/*.json extension/*.css "$extension_root/"
install -m644 extension/schemas/*.xml "$extension_root/schemas/"
glib-compile-schemas --strict "$extension_root/schemas"
if [[ -d .deps/root ]]; then
    # Only the OCR runtime is bundled; GTK/GStreamer are provided by Ubuntu.
    for lib in libtesseract.so.5 liblept.so.5; do
        cp -L ".deps/root/usr/lib/x86_64-linux-gnu/$lib" "$install_root/lib/$lib"
    done
    model=$(find .deps/root/usr/share/tesseract-ocr -name eng.traineddata -print -quit)
    install -m644 "$model" "$install_root/tessdata/eng.traineddata"
    mkdir -p "$install_root/licenses"
    for package in libtesseract5 liblept5 tesseract-ocr-eng; do
        install -m644 ".deps/root/usr/share/doc/$package/copyright" "$install_root/licenses/$package.txt"
    done
fi
# Python's shell quoting handles spaces and special characters in the home path.
python3 - "$install_root" "$bin_dir" "$install_prefix/share" <<'PY'
import pathlib, shlex, sys
root, bindir, share = map(pathlib.Path, sys.argv[1:])
binary = bindir / 'area-translator'
binary.write_text('#!/bin/sh\n'
    f'export LD_LIBRARY_PATH={shlex.quote(str(root / "lib"))}${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}\n'
    f'if [ -f {shlex.quote(str(root / "tessdata/eng.traineddata"))} ]; then export TESSDATA_PREFIX={shlex.quote(str(root / "tessdata"))}; fi\n'
    'export OMP_THREAD_LIMIT=1\n'
    f'exec {shlex.quote(str(root / "bin/area-translator"))} "$@"\n')
ui = bindir / 'area-translator-ui'
ui.write_text('#!/bin/sh\nexec gjs -m ' + shlex.quote(str(root / 'ui/app.js')) + ' "$@"\n')
binary.chmod(0o755); ui.chmod(0o755)
# Desktop entry escaping differs from POSIX shell quoting.
def desktop_quote(s):
    return '"' + str(s).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$') + '"'
(share / 'applications/io.github.areatranslator.App.desktop').write_text(
    '[Desktop Entry]\nType=Application\nName=Tradutor de área\nComment=Traduza o texto de uma região da tela\n'
    f'Exec={desktop_quote(ui)}\nIcon=accessories-dictionary\nTerminal=false\nCategories=Utility;Accessibility;\n')
(share / 'dbus-1/services/io.github.areatranslator.Service.service').write_text(
    '[D-BUS Service]\nName=io.github.areatranslator.Service\nExec=' + desktop_quote(binary) + '\n')
PY
if command -v update-desktop-database >/dev/null; then update-desktop-database "$install_prefix/share/applications"; fi
echo 'Instalado. Ative a extensão com: gnome-extensions enable area-translator@local'
echo 'Se o GNOME ainda não encontrar a extensão, saia da sessão e entre novamente.'
echo "Abra “Tradutor de área” no menu de aplicativos ou execute: $bin_dir/area-translator-ui"
