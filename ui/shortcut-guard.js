// No GTK, capture or polling. Release the global key when its Rust D-Bus owner exits.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {watchShortcut} from './shortcut-state.js';
if (ARGV.length !== 1 || !/^:[0-9]+\.[0-9]+$/.test(ARGV[0])) throw new Error('Expected unique D-Bus owner');
const loop = new GLib.MainLoop(null, false);
const cleanup = watchShortcut(Gio.DBus.session, ARGV[0], () => loop.quit());
GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, 2, () => { cleanup(); loop.quit(); return GLib.SOURCE_REMOVE; });
GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, 15, () => { cleanup(); loop.quit(); return GLib.SOURCE_REMOVE; });
loop.run();
cleanup();
