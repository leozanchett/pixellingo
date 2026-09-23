// Mock only: run on an isolated bus so desktop notifications never reach the user.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
if (GLib.getenv('AREA_TRANSLATOR_ISOLATED_TEST') !== '1') throw new Error('Requires isolated D-Bus');
const bus = Gio.DBus.session;
const name = 'io.github.areatranslator.Service';
const requestName = value => bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
    'org.freedesktop.DBus', 'RequestName', new GLib.Variant('(su)', [value, 0]), null, Gio.DBusCallFlags.NONE, 1000, null);
const notices = [];
const notifications = Gio.DBusExportedObject.wrapJSObject(`<node><interface name="org.freedesktop.Notifications">
<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
<arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
<arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
</interface></node>`, {Notify: (_app, _id, _icon, _title, body) => { notices.push(body); return 1; }});
notifications.export(bus, '/org/freedesktop/Notifications'); requestName('org.freedesktop.Notifications');
const run = () => new Promise((resolve, reject) => {
    const child = Gio.Subprocess.new(['gjs', '-m', 'ui/refresh.js'], Gio.SubprocessFlags.STDERR_PIPE);
    child.communicate_utf8_async(null, null, (process, result) => {
        try { process.communicate_utf8_finish(result); if (!process.get_successful()) throw new Error('Client failed'); resolve(); }
        catch (error) { reject(error); }
    });
});
let requests = 0;
let fail = true;
const service = Gio.DBusExportedObject.wrapJSObject(`<node><interface name="${name}"><method name="Refresh"/></interface></node>`, {
    RefreshAsync: (_args, invocation) => {
        requests++;
        if (fail) invocation.return_dbus_error(`${name}.Error`, 'Retome a captura antes de traduzir.');
        else invocation.return_value(null);
    },
});
const loop = new GLib.MainLoop(null, false);
let failure;
(async () => {
    await run();
    if (notices.length !== 1 || !notices[0].includes('selecione a área')) throw new Error('Missing-service notification absent');
    const [owner] = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
        'NameHasOwner', new GLib.Variant('(s)', [name]), null, Gio.DBusCallFlags.NONE, 1000, null).deepUnpack();
    if (owner) throw new Error('Shortcut must not activate service');
    service.export(bus, '/io/github/areatranslator/Service'); requestName(name);
    await run();
    if (notices.length !== 2 || !notices[1].includes('Retome')) throw new Error('Rejected-request notification absent');
    fail = false; await run();
    if (requests !== 2 || notices.length !== 2) throw new Error('Success must deliver exactly one request without warning');
    print('Shortcut client: inactive/rejected requests notify; successful request delivered once.');
})().catch(error => { failure = error; }).finally(() => loop.quit());
loop.run();
service.unexport(); notifications.unexport();
if (failure) throw failure;
