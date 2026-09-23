import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {placeSubtitle, placeWindowSubtitle, regionOnScreen, subtitleHeight} from './geometry.js';

const BUS = 'io.github.areatranslator.Service';
const PATH = '/io/github/areatranslator/Service';
const XML = `<node><interface name="io.github.areatranslator.Overlay"><method name="GetMonitors"><arg type="s" direction="out"/></method><method name="GetVersion"><arg type="u" direction="out"/></method></interface></node>`;

export default class AreaTranslator extends Extension {
    enable() {
        this._alive = true;
        this._settings = this.getSettings();
        this._snapshot = null;
        this._text = '';
        this._preferred = null;
        this._editing = false;
        this._generation = -1;
        this._revision = -1;
        this._panel = new PanelMenu.Button(0, 'Tradutor de área');
        this._panel.add_child(new St.Icon({icon_name: 'accessories-dictionary-symbolic', style_class: 'system-status-icon'}));
        Main.panel.addToStatusArea(this.uuid, this._panel);
        this._stateItem = new PopupMenu.PopupMenuItem('Selecione uma área para começar.', {reactive: false});
        this._panel.menu.addMenuItem(this._stateItem);
        this._panel.menu.addAction('Selecionar área / configurações', () => {
            Gio.Subprocess.new(['gjs', '-m', GLib.build_filenamev([GLib.get_user_data_dir(), 'area-translator', 'ui', 'app.js'])], Gio.SubprocessFlags.NONE);
        });
        this._toggle = this._panel.menu.addAction('Pausar / retomar', () => this._togglePause());
        this._panel.menu.addAction('Reposicionar legenda', () => this._startEditing());
        this._background = new PopupMenu.PopupSwitchMenuItem('Fundo translúcido', this._settings.get_boolean('background'));
        this._background.connect('toggled', (_item, value) => { this._settings.set_boolean('background', value); this._style(); });
        this._panel.menu.addMenuItem(this._background);
        this._panel.menu.addAction('Encerrar captura', () => this._call('Stop'));
        this._label = new St.Label({style_class: 'area-translator-subtitle', reactive: false, can_focus: false, visible: false});
        this._label.clutter_text.set_line_wrap(true);
        this._label.clutter_text.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR);
        this._label.clutter_text.set_ellipsize(Pango.EllipsizeMode.END);
        this._label.clutter_text.set_line_alignment(Pango.Alignment.CENTER);
        // Shell chrome stays above fullscreen windows; it must not reserve a strut
        // or enter the input region during normal use.
        Main.layoutManager.addTopChrome(this._label, {affectsInputRegion: false, affectsStruts: false, trackFullscreen: false});
        this._style();
        this._label.connect('button-press-event', (_actor, event) => {
            if (!this._editing) return Clutter.EVENT_PROPAGATE;
            const [x, y] = event.get_coords();
            this._drag = {x, y, originX: this._label.x, originY: this._label.y};
            return Clutter.EVENT_STOP;
        });
        this._label.connect('motion-event', (_actor, event) => {
            if (!this._editing || !this._drag) return Clutter.EVENT_PROPAGATE;
            const [x, y] = event.get_coords();
            this._preferred = {x: this._drag.originX + x - this._drag.x, y: this._drag.originY + y - this._drag.y};
            this._render();
            return Clutter.EVENT_STOP;
        });
        this._label.connect('button-release-event', () => {
            if (!this._editing) return Clutter.EVENT_PROPAGATE;
            this._finishEditing();
            return Clutter.EVENT_STOP;
        });
        this._exported = Gio.DBusExportedObject.wrapJSObject(XML, {
            GetVersion: () => 2,
            GetMonitors: () => JSON.stringify(Main.layoutManager.monitors.map((m, index) => ({
                x: m.x, y: m.y, width: m.width, height: m.height, name: `Monitor ${index + 1}`,
            }))),
        });
        this._exported.export(Gio.DBus.session, '/io/github/areatranslator/Overlay');
        this._signal = Gio.DBus.session.signal_subscribe(BUS, BUS, null, PATH, null, Gio.DBusSignalFlags.NONE,
            (_bus, _sender, _path, _interface, signal, parameters) => {
                if (!this._alive) return;
                if (signal === 'StatusChanged') this._update(JSON.parse(parameters.deepUnpack()[0]));
                if (signal === 'TranslationChanged') {
                    const [generation, revision, text] = parameters.deepUnpack();
                    if (generation < this._generation || (generation === this._generation && revision < this._revision)) return;
                    this._generation = generation; this._revision = revision; this._text = text;
                    this._render();
                }
            });
        this._watch = Gio.bus_watch_name(Gio.BusType.SESSION, BUS, Gio.BusNameWatcherFlags.NONE,
            () => this._call('GetStatus', result => this._update(JSON.parse(result[0]))),
            () => { if (this._alive) { this._text = ''; this._snapshot = null; this._generation = -1; this._revision = -1; this._render(); } });
        this._monitorsChanged = Main.layoutManager.connect('monitors-changed', () => {
            this._preferred = null;
            this._call('Stop');
            this._stateItem.label.text = 'Monitores alterados: selecione a área novamente.';
        });
        this._overviewShowing = Main.overview.connect('showing', () => this._label.hide());
        this._overviewHidden = Main.overview.connect('hidden', () => this._render());
        this._sessionChanged = Main.sessionMode.connect('updated', () => {
            if (Main.sessionMode.isLocked) {
                this._label.hide();
                if (['running', 'retrying'].includes(this._snapshot?.state)) this._call('Pause');
            } else this._render();
        });
        Main.wm.addKeybinding('toggle-shortcut', this._settings, Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL, () => this._togglePause());
    }

    _call(method, callback = null) {
        Gio.DBus.session.call(BUS, PATH, BUS, method, null, null, Gio.DBusCallFlags.NO_AUTO_START, 10000, null,
            (connection, result) => {
                try { const value = connection.call_finish(result).deepUnpack(); if (this._alive) callback?.(value); }
                catch (error) { if (this._alive) this._stateItem.label.text = `Serviço indisponível: ${error.message}`; }
            });
    }
    _update(snapshot) {
        if (!this._alive) return;
        if (snapshot.generation < this._generation) return;
        if (snapshot.source_type !== this._snapshot?.source_type
            || JSON.stringify(snapshot.region) !== JSON.stringify(this._snapshot?.region)
            || JSON.stringify(snapshot.monitor) !== JSON.stringify(this._snapshot?.monitor)) this._preferred = null;
        this._snapshot = snapshot;
        this._generation = snapshot.generation;
        this._revision = snapshot.revision;
        this._text = snapshot.translation;
        this._stateItem.label.text = snapshot.message;
        this._toggle.label.text = ['paused', 'blocked'].includes(snapshot.state) ? 'Retomar tradução' : 'Pausar tradução';
        this._render();
    }
    _togglePause() {
        if (!this._snapshot?.region) return;
        this._call(['paused', 'blocked'].includes(this._snapshot.state) ? 'Resume' : 'Pause');
    }
    _style() {
        this._label?.set_style_class_name(`area-translator-subtitle${this._settings.get_boolean('background') ? ' with-background' : ''}${this._editing ? ' repositioning' : ''}`);
    }
    _render() {
        const s = this._snapshot;
        if (!s?.region || !s.monitor || !s.frame_size || Main.overview.visible || Main.sessionMode.isLocked
            || (!this._editing && (!this._text || !['running', 'retrying'].includes(s.state)))) {
            this._label.hide(); return;
        }
        const windowCapture = s.source_type === 'window';
        const region = windowCapture ? null : regionOnScreen(s.region, s.monitor, s.frame_size);
        const width = Math.min(900, s.monitor.width - 32);
        this._label.text = this._editing ? 'Arraste a legenda e solte para confirmar' : this._text;
        // Release the previous height before measuring the wrapped text. Long
        // translations grow into the available space instead of a fixed 92px box.
        this._label.set_size(width, -1);
        const [, naturalHeight] = this._label.get_preferred_height(width);
        const height = windowCapture ? naturalHeight : subtitleHeight(region, s.monitor, naturalHeight);
        if (!height) { this._label.hide(); return; }
        const position = windowCapture ? placeWindowSubtitle(s.monitor, width, height, this._preferred)
            : placeSubtitle(region, s.monitor, width, height, this._preferred);
        if (!position) { this._label.hide(); return; }
        this._label.set_size(position.width, position.height);
        this._label.set_position(Math.round(position.x), Math.round(position.y));
        this._label.show();
    }
    _startEditing() {
        if (!this._snapshot?.region || this._editing) return;
        this._resumeAfterEdit = ['running', 'retrying'].includes(this._snapshot.state);
        this._call('Pause', () => {
            this._editing = true;
            this._label.reactive = true;
            Main.layoutManager.removeChrome(this._label);
            Main.layoutManager.addTopChrome(this._label, {affectsInputRegion: true, affectsStruts: false, trackFullscreen: false});
            this._style(); this._render();
            this._editTimeout = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 15, () => {
                this._editTimeout = 0; this._finishEditing(); return GLib.SOURCE_REMOVE;
            });
        });
    }
    _finishEditing() {
        if (!this._editing) return;
        this._editing = false; this._drag = null;
        if (this._editTimeout) { GLib.source_remove(this._editTimeout); this._editTimeout = 0; }
        this._label.reactive = false;
        Main.layoutManager.removeChrome(this._label);
        Main.layoutManager.addTopChrome(this._label, {affectsInputRegion: false, affectsStruts: false, trackFullscreen: false});
        this._style(); this._render();
        if (this._resumeAfterEdit) this._call('Resume');
    }
    disable() {
        this._call('Stop');
        this._alive = false;
        if (this._editTimeout) GLib.source_remove(this._editTimeout);
        Main.wm.removeKeybinding('toggle-shortcut');
        Main.layoutManager.disconnect(this._monitorsChanged);
        Main.overview.disconnect(this._overviewShowing);
        Main.overview.disconnect(this._overviewHidden);
        Main.sessionMode.disconnect(this._sessionChanged);
        Gio.DBus.session.signal_unsubscribe(this._signal);
        Gio.bus_unwatch_name(this._watch);
        this._exported.unexport();
        Main.layoutManager.removeChrome(this._label);
        this._label.destroy(); this._panel.destroy();
        this._settings = null;
        this._label = null;
    }
}
