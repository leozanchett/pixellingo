#!/usr/bin/env bash
set -euo pipefail
gdbus call --session --dest io.github.areatranslator.Service --object-path /io/github/areatranslator/Service \
    --method io.github.areatranslator.Service.Stop >/dev/null 2>&1 || true
gnome-extensions disable area-translator@local 2>/dev/null || true
rm -rf -- "$HOME/.local/share/area-translator" "$HOME/.local/share/gnome-shell/extensions/area-translator@local"
rm -f -- "$HOME/.local/bin/area-translator" "$HOME/.local/bin/area-translator-ui" \
    "$HOME/.local/share/applications/io.github.areatranslator.App.desktop" \
    "$HOME/.local/share/dbus-1/services/io.github.areatranslator.Service.service"
echo 'Aplicação removida. A chave permanece no chaveiro; remova “Area Translator — Google Cloud” em Senhas e chaves se desejar.'
