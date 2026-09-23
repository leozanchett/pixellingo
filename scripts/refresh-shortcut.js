// GNOME custom shortcuts take effect without reloading Shell.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
const path = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/pixellingo-refresh/';
const root = new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys'});
const settings = at => new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys.custom-keybinding', path: at});
function install(command) {
    const entries = root.get_strv('custom-keybindings');
    const own = settings(path);
    own.set_string('name', 'PixelLingo — Atualizar tradução');
    own.set_string('command', command);
    if (!entries.includes(path)) {
        own.set_string('binding', '<Super><Shift>r');
        root.set_strv('custom-keybindings', [...entries, path]);
    }
    Gio.Settings.sync();
}
function remove() {
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
    root.set_strv('custom-keybindings', [other]);
    install("'/tmp/path with spaces/refresh'");
    if (settings(path).get_string('binding') !== '<Super><Shift>r') throw new Error('Missing shortcut');
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
