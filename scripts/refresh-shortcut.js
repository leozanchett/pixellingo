// GNOME custom shortcuts take effect without reloading Shell.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
const path = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/pixellingo-refresh/';
const root = new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys'});
const settings = at => new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys.custom-keybinding', path: at});
function removeLegacyDialogueShortcut() {
    const legacy = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/pixellingo-dialogue/';
    root.set_strv('custom-keybindings', root.get_strv('custom-keybindings').filter(entry => entry !== legacy));
    for (const key of ['name', 'command', 'binding']) settings(legacy).reset(key);
}
function install(command) {
    removeLegacyDialogueShortcut();
    const entries = root.get_strv('custom-keybindings');
    const own = settings(path);
    own.set_string('name', 'PixelLingo — Atualizar tradução');
    own.set_string('command', command);
    if (!entries.includes(path)) {
        own.set_string('binding', '<Control>a');
        root.set_strv('custom-keybindings', [...entries, path]);
    }
    if (own.get_string('binding') === '<Super><Shift>r') own.set_string('binding', '<Control>a');
    Gio.Settings.sync();
}
function remove() {
    removeLegacyDialogueShortcut();
    root.set_strv('custom-keybindings', root.get_strv('custom-keybindings').filter(entry => entry !== path));
    for (const key of ['name', 'command', 'binding']) settings(path).reset(key);
    Gio.Settings.sync();
}
if (ARGV[0] === 'install' && ARGV[1]) {
    install(GLib.shell_quote(ARGV[1]));
} else if (ARGV[0] === 'remove') {
    remove();
} else if (ARGV[0] === 'test' && GLib.getenv('GSETTINGS_BACKEND') === 'memory') {
    const other = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/other/';
    const legacy = path.replace('pixellingo-refresh', 'pixellingo-dialogue');
    root.set_strv('custom-keybindings', [other, legacy]);
    settings(legacy).set_string('binding', '<Control>s');
    install("'/tmp/path with spaces/refresh'");
    if (root.get_strv('custom-keybindings').includes(legacy) || settings(legacy).get_string('binding')) throw new Error('Legacy dialogue shortcut retained');
    if (settings(path).get_string('binding') !== '<Control>a') throw new Error('Missing shortcut');
    settings(path).set_string('binding', '<Super><Alt>r');
    install("'/tmp/new refresh'");
    if (root.get_strv('custom-keybindings').length !== 2) throw new Error('Duplicate shortcut');
    if (settings(path).get_string('binding') !== '<Super><Alt>r') throw new Error('User binding overwritten');
    settings(path).set_string('binding', '');
    install("'/tmp/new refresh'");
    if (settings(path).get_string('binding') !== '') throw new Error('Disabled binding reenabled');
    remove();
    if (JSON.stringify(root.get_strv('custom-keybindings')) !== JSON.stringify([other])) throw new Error('Other shortcuts modified');
    print('Shortcut install/update/remove preserves user settings.');
} else {
    throw new Error('Use install <executable>, remove, or test with GSETTINGS_BACKEND=memory.');
}
