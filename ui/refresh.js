// Manual shortcut: a short-lived D-Bus client, never GTK or a new service.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
const loop = new GLib.MainLoop(null, false);
Gio.DBus.session.call('io.github.areatranslator.Service',
    '/io/github/areatranslator/Service', 'io.github.areatranslator.Service',
    'Refresh', null, null, Gio.DBusCallFlags.NO_AUTO_START, 5000, null,
    (connection, result) => {
        try { connection.call_finish(result); }
        catch (_) { /* No active session: do not open a window over the game. */ }
        loop.quit();
    });
loop.run();
