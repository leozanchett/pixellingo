// Isolated settings and synthetic GTK windows; no credentials, capture or API.
import Gtk from 'gi://Gtk?version=4.0';
import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {SHORTCUT_PATH, DEFAULT_SHORTCUT, acceleratorFromKey, readShortcut,
    saveShortcut, shortcutLabel, showShortcutDialog} from '../ui/shortcut.js';

if (GLib.getenv('GSETTINGS_BACKEND') !== 'memory') throw new Error('Requires memory settings backend');
Gtk.init();
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const rejects = callback => {
    let rejected = false;
    try { callback(); } catch (_) { rejected = true; }
    assert(rejected, 'Expected invalid/conflicting shortcut to be rejected');
};
const root = new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys'});
const settings = path => new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys.custom-keybinding', path});
const other = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/test-other/';
root.set_strv('custom-keybindings', [other]);
settings(SHORTCUT_PATH).set_string('command', '/test/refresh');
settings(other).set_string('name', 'Atalho existente');
settings(other).set_string('binding', '<Control><Alt>k');
assert(acceleratorFromKey(Gdk.KEY_Shift_L, 0) === null, 'Modifier alone must wait');
rejects(() => acceleratorFromKey(Gdk.KEY_a, 0));
rejects(() => acceleratorFromKey(Gdk.KEY_A, Gdk.ModifierType.SHIFT_MASK));
saveShortcut('F8');
assert(readShortcut() === 'F8', 'Function key should persist');
assert(!root.get_strv('custom-keybindings').includes(SHORTCUT_PATH), 'Saving while idle must not reserve the key');
rejects(() => saveShortcut('<Control><Alt>k'));
assert(readShortcut() === 'F8', 'Conflict must preserve previous binding');
const wm = new Gio.Settings({schema_id: 'org.gnome.desktop.wm.keybindings'});
wm.set_strv('close', ['<Alt>F4']);
rejects(() => saveShortcut('<Alt>F4'));
saveShortcut('');
assert(shortcutLabel() === 'Desativado', 'Disable should persist');
saveShortcut(DEFAULT_SHORTCUT);
assert(readShortcut().includes('Control') && readShortcut().endsWith('a'), 'Restore default');
assert(settings(other).get_string('binding') === '<Control><Alt>k', 'Other binding changed');

const parent = new Gtk.Window({title: 'PixelLingo — Teste de configuração', default_width: 560, default_height: 300});
parent.present();
let saved = 0;
let dialog = showShortcutDialog(parent, () => saved++);
const loop = new GLib.MainLoop(null, false);
let failure;
const click = (widget, label) => {
    if (widget instanceof Gtk.Button && widget.label === label) { widget.emit('clicked'); return true; }
    for (let child = widget.get_first_child(); child; child = child.get_next_sibling())
        if (click(child, label)) return true;
    return false;
};
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
    try {
        const controllers = dialog.observe_controllers();
        let key;
        for (let i = 0; i < controllers.get_n_items(); i++) {
            const controller = controllers.get_item(i);
            if (controller.get_name() === 'pixellingo-shortcut-recorder') key = controller;
        }
        assert(key, 'Keyboard recorder missing');
        key.emit('key-pressed', Gdk.KEY_F8, 0, 0);
        assert(click(dialog, 'Salvar'), 'Save button missing');
        assert(saved === 1 && readShortcut() === 'F8', `Recording must save selected key: saved=${saved}, binding=${readShortcut()}`);
        dialog = showShortcutDialog(parent, () => saved++);
        assert(click(dialog, 'Desativar'), 'Disable button missing');
        click(dialog, 'Cancelar');
        assert(saved === 1 && readShortcut() === 'F8', 'Cancel must discard pending changes');
        dialog = showShortcutDialog(parent, () => saved++);
        click(dialog, 'Desativar'); click(dialog, 'Salvar');
        assert(saved === 2 && readShortcut() === '', 'Disable must apply on save');
        dialog = showShortcutDialog(parent, () => saved++);
        click(dialog, 'Restaurar padrão'); click(dialog, 'Salvar');
        assert(saved === 3 && readShortcut().endsWith('a'), 'Restore must apply on save');
        dialog = showShortcutDialog(parent, () => saved++);
        print('Shortcut validation, conflicts, recording, cancel, disable and restore passed.');
    } catch (e) { failure = e; }
    return GLib.SOURCE_REMOVE;
});
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => {
    dialog.close(); parent.close(); loop.quit(); return GLib.SOURCE_REMOVE;
});
loop.run();
if (failure) throw failure;
