// Same-process memory settings, separate D-Bus connections: never grabs desktop keys.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {SHORTCUT_PATH, shortcutSettings, watchShortcut} from '../ui/shortcut-state.js';
if (GLib.getenv('GSETTINGS_BACKEND') !== 'memory' || GLib.getenv('AREA_TRANSLATOR_ISOLATED_TEST') !== '1')
    throw new Error('Requires isolated bus and memory settings');
const root = new Gio.Settings({schema_id: 'org.gnome.settings-daemon.plugins.media-keys'});
const other = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/other/';
root.set_strv('custom-keybindings', [other, SHORTCUT_PATH]);
shortcutSettings().set_string('command', '/test/refresh');
shortcutSettings().set_string('binding', '<Control>a');
const connected = () => Gio.DBusConnection.new_for_address_sync(GLib.getenv('DBUS_SESSION_BUS_ADDRESS'),
    Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, null, null);
const service = connected();
const observer = connected();
const name = 'io.github.areatranslator.Service';
let initial;
const object = Gio.DBusExportedObject.wrapJSObject(`<node><interface name="${name}">
<method name="GetStatus"><arg type="s" direction="out"/></method>
<signal name="StatusChanged"><arg type="s"/></signal></interface></node>`, {
    GetStatusAsync: (_args, invocation) => { initial = invocation; },
});
object.export(service, '/io/github/areatranslator/Service');
let gone = false;
const cleanup = watchShortcut(observer, service.get_unique_name(), () => { gone = true; });
const sleep = ms => new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => { resolve(); return GLib.SOURCE_REMOVE; }));
const assert = (value, message) => { if (!value) throw new Error(message); };
const active = () => root.get_strv('custom-keybindings').includes(SHORTCUT_PATH);
const emit = (state, region = {x: 0, y: 0, width: 100, height: 100}) =>
    object.emit_signal('StatusChanged', new GLib.Variant('(s)', [JSON.stringify({state, region})]));
const loop = new GLib.MainLoop(null, false);
let failure;
(async () => {
    assert(!active(), 'Startup must release a stale registration');
    for (let i = 0; !initial && i < 100; i++) await sleep(10);
    assert(initial, 'Initial snapshot not requested');
    emit('paused'); await sleep(30);
    initial.return_value(new GLib.Variant('(s)', [JSON.stringify({state: 'running', region: {}})]));
    await sleep(30); assert(!active(), 'Late initial snapshot reactivated a paused key');
    for (const state of ['running', 'paused', 'running', 'blocked', 'running', 'idle', 'opening', 'selecting', 'error']) {
        emit(state); await sleep(30);
        assert(active() === (state === 'running'), `Wrong registration for ${state}`);
        assert(shortcutSettings().get_string('binding') === '<Control>a', 'Saved combination lost');
        assert(root.get_strv('custom-keybindings').includes(other), 'Other shortcut removed');
    }
    emit('running', null); await sleep(30); assert(!active(), 'Missing region must release key');
    emit('running'); await sleep(30); assert(active(), 'Running capture did not reserve key');
    shortcutSettings().set_string('binding', 'F8');
    emit('idle'); await sleep(30);
    emit('running'); await sleep(30);
    assert(shortcutSettings().get_string('binding') === 'F8', 'Custom choice overwritten');
    service.close_sync(null); await sleep(60);
    assert(gone && !active(), 'Unexpected service exit must release key');
    assert(shortcutSettings().get_string('binding') === 'F8', 'Exit erased preference');
    assert(JSON.stringify(root.get_strv('custom-keybindings')) === JSON.stringify([other]), 'Cleanup touched other entries');
    cleanup();
    print('Shortcut lifecycle: idle/pause/stop/error/owner loss release key; resume restores saved combination; stale status ignored.');
})().catch(e => { failure = e; }).finally(() => loop.quit());
loop.run();
cleanup(); observer.close_sync(null);
if (failure) throw failure;
