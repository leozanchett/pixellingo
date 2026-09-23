// Manual shortcut: a short-lived D-Bus client, never GTK or a new service.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
const loop = new GLib.MainLoop(null, false);
Gio.DBus.session.call('io.github.areatranslator.Service',
    '/io/github/areatranslator/Service', 'io.github.areatranslator.Service',
    'Refresh', null, null, Gio.DBusCallFlags.NO_AUTO_START, 5000, null,
    (connection, result) => {
        try { connection.call_finish(result); loop.quit(); }
        catch (error) {
            const missing = /ServiceUnknown|NameHasNoOwner/.test(error.message);
            const message = missing ? 'Abra o tradutor e selecione a área antes de usar o atalho.'
                : error.message.replace(/^GDBus\.Error:[^:]+:\s*/, '').slice(0, 240);
            printerr(`PixelLingo: ${message}`);
            // Explain failed shortcuts without opening GTK or stealing game focus.
            Gio.DBus.session.call('org.freedesktop.Notifications', '/org/freedesktop/Notifications',
                'org.freedesktop.Notifications', 'Notify',
                new GLib.Variant('(susssasa{sv}i)', ['PixelLingo', 0, 'accessories-dictionary',
                    'Não foi possível traduzir', GLib.markup_escape_text(message, -1), [], {}, 5000]),
                null, Gio.DBusCallFlags.NO_AUTO_START, 1500, null, (bus, notification) => {
                    try { bus.call_finish(notification); } catch (_) { /* stderr remains available. */ }
                    loop.quit();
                });
        }
    });
loop.run();
