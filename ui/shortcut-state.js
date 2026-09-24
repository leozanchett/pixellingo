// Saved accelerator and active registration are independent: idle must release the key.
import Gio from 'gi://Gio';
export const SHORTCUT_PATH = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/pixellingo-refresh/';
export const DEFAULT_SHORTCUT = '<Control>a';
export const shortcutSettings = () => new Gio.Settings({
    schema_id: 'org.gnome.settings-daemon.plugins.media-keys.custom-keybinding', path: SHORTCUT_PATH});
export function setShortcutActive(active) {
    const root = new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys'});
    const entries = root.get_strv('custom-keybindings');
    // Do not resurrect an uninstalled entry.
    active = active && Boolean(shortcutSettings().get_string('command'));
    if (active === entries.includes(SHORTCUT_PATH)) return;
    root.set_strv('custom-keybindings', active ? [...entries, SHORTCUT_PATH]
        : entries.filter(path => path !== SHORTCUT_PATH));
    Gio.Settings.sync();
}
export function watchShortcut(bus, owner, onGone) {
    let alive = true;
    let revision = 0;
    setShortcutActive(false);
    const update = text => {
        let active = false;
        try { const s = JSON.parse(text); active = s.state === 'running' && Boolean(s.region); }
        catch (_) { /* Invalid status must release the key. */ }
        setShortcutActive(active);
    };
    const signal = bus.signal_subscribe(owner, 'io.github.areatranslator.Service', 'StatusChanged',
        '/io/github/areatranslator/Service', null, Gio.DBusSignalFlags.NONE,
        (_bus, _sender, _path, _interface, _signal, args) => {
            if (!alive) return;
            revision++; update(args.deepUnpack()[0]);
        });
    const watch = Gio.bus_watch_name_on_connection(bus, owner, Gio.BusNameWatcherFlags.NONE,
        () => {}, () => { if (alive) { cleanup(); onGone(); } });
    const cleanup = () => {
        if (!alive) return;
        alive = false;
        bus.signal_unsubscribe(signal);
        Gio.bus_unwatch_name(watch);
        setShortcutActive(false);
    };
    bus.call(owner, '/io/github/areatranslator/Service', 'io.github.areatranslator.Service',
        'GetStatus', null, null, Gio.DBusCallFlags.NO_AUTO_START, 5000, null, (connection, result) => {
            try {
                const [text] = connection.call_finish(result).deepUnpack();
                if (alive && revision === 0) update(text);
            } catch (_) { /* No status means no shortcut. */ }
        });
    return cleanup;
}
