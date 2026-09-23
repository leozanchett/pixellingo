#!/usr/bin/env python3
"""Check real daemon → portal requests on an isolated bus, without screen/network access."""
import ast
import json
import os
import signal
import subprocess
import threading
import time
from pathlib import Path

import gi
gi.require_version('Gio', '2.0')
from gi.repository import Gio, GLib

assert os.environ.get('AREA_TRANSLATOR_ISOLATED_TEST') == '1', 'Requires dbus-run-session'
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
root = '/org/freedesktop/portal/desktop'
interface = 'org.freedesktop.portal.ScreenCast'
xml = '''<node><interface name="org.freedesktop.portal.ScreenCast">
<property name="version" type="u" access="read"/>
<property name="AvailableSourceTypes" type="u" access="read"/>
<property name="AvailableCursorModes" type="u" access="read"/>
<method name="CreateSession"><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
<method name="SelectSources"><arg type="o" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
<method name="Start"><arg type="o" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
</interface></node>'''
session_info = Gio.DBusNodeInfo.new_for_xml('''<node><interface name="org.freedesktop.portal.Session">
<property name="version" type="u" access="read"/><method name="Close"/>
</interface></node>''').interfaces[0]
events = []
available = 3
created = 0


def session_method(_bus, _sender, _path, _interface, method, _args, invocation):
    assert method == 'Close'
    events.append(('closed', None))
    invocation.return_value(None)


def portal_method(connection, sender, _path, _interface, method, parameters, invocation):
    global created
    values = parameters.unpack()
    options = values[-1]
    sender_id = sender[1:].replace('.', '_')
    request = f'{root}/request/{sender_id}/{options["handle_token"]}'
    result = {}
    code = 0
    if method == 'CreateSession':
        created += 1
        session = f'{root}/session/{sender_id}/{options["session_handle_token"]}'
        connection.register_object(session, session_info, session_method,
                                   lambda *_: GLib.Variant('u', 1), None)
        result['session_handle'] = GLib.Variant('s', session)
    elif method == 'SelectSources':
        events.append(('sources', options))
    elif method == 'Start':
        # Simulate user cancelling the chooser. No PipeWire or cloud calls.
        code = 1
    invocation.return_value(GLib.Variant('(o)', (request,)))

    def respond():
        connection.emit_signal(sender, request, 'org.freedesktop.portal.Request', 'Response',
                               GLib.Variant('(ua{sv})', (code, result)))
        return GLib.SOURCE_REMOVE
    GLib.timeout_add(20, respond)


def prop(_bus, _sender, _path, _iface, name):
    return GLib.Variant('u', {'version': 5, 'AvailableSourceTypes': available,
                             'AvailableCursorModes': 1}[name])


bus.register_object(root, Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0], portal_method, prop, None)
bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
              'RequestName', GLib.Variant('(su)', ('org.freedesktop.portal.Desktop', 0)),
              None, Gio.DBusCallFlags.NONE, 5000, None)
loop = GLib.MainLoop()
failure = []


def exercise():
    global available
    daemon = None
    try:
        Path('.deps').mkdir(exist_ok=True)
        with open('.deps/portal-service.log', 'w') as log:
            daemon = subprocess.Popen(['target/debug/area-translator'], stdout=log, stderr=log)
        base = ['gdbus', 'call', '--session', '--dest', 'io.github.areatranslator.Service',
                '--object-path', '/io/github/areatranslator/Service', '--method']

        def call(method, *args):
            return subprocess.check_output(base + ['io.github.areatranslator.Service.' + method, *args],
                                           text=True, stderr=subprocess.DEVNULL, timeout=5)

        def status():
            return json.loads(ast.literal_eval(call('GetStatus'))[0])

        for _ in range(50):
            owner = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                                  'org.freedesktop.DBus', 'NameHasOwner',
                                  GLib.Variant('(s)', ('io.github.areatranslator.Service',)),
                                  None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
            if owner:
                break
            assert daemon.poll() is None, 'Test daemon exited before owning its bus name'
            time.sleep(.1)
        assert owner, 'Test daemon did not start'
        call('SetApiKey', 'test-only-no-network')
        for method, expected in [('BeginWindowSelection', 2), ('BeginSelection', 1)]:
            before = len(events)
            call(method)
            for _ in range(60):
                state = status()
                if state['state'] == 'error':
                    break
                time.sleep(.1)
            assert state['state'] == 'error', state['state']
            source = next(value for kind, value in events[before:] if kind == 'sources')
            assert source['types'] == expected, source
            assert source['multiple'] is False and source['cursor_mode'] == 1
            assert source['persist_mode'] == 0
            assert any(kind == 'closed' for kind, _ in events[before:])
            assert state['region'] is None and state['source_type'] is None
            assert state['api_count'] == 0
        available = 1
        bus.emit_signal(None, root, 'org.freedesktop.DBus.Properties', 'PropertiesChanged',
                        GLib.Variant('(sa{sv}as)', (interface, {'AvailableSourceTypes': GLib.Variant('u', 1)}, [])))
        before = created
        call('BeginWindowSelection')
        for _ in range(60):
            state = status()
            if state['state'] == 'error':
                break
            time.sleep(.1)
        assert state['state'] == 'error' and created == before
        assert 'não oferece' in state['message']
        print('Portal contract passed: window/monitor source, cancellation cleanup, unsupported window, no API calls.')
    except BaseException as error:
        failure.append(error)
    finally:
        if daemon:
            daemon.send_signal(signal.SIGINT)
            daemon.wait(timeout=5)
        GLib.idle_add(loop.quit)


thread = threading.Thread(target=exercise, daemon=True)
thread.start()
loop.run()
thread.join(timeout=5)
if failure:
    raise failure[0]
