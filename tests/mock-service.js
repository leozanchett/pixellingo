// Synthetic overlay test. No capture, OCR, credentials or network calls.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';
const name = 'io.github.areatranslator.Service';
const xml = `<node><interface name="${name}">
<method name="GetStatus"><arg type="s" direction="out"/></method>
<method name="Stop"/><method name="Pause"/><method name="Resume"/>
<method name="GetClicks"><arg type="u" direction="out"/></method>
<method name="LongSubtitle"/><method name="ShortSubtitle"/>
<method name="WindowSource"/>
<signal name="StatusChanged"><arg type="s"/></signal>
<signal name="TranslationChanged"><arg type="t"/><arg type="t"/><arg type="s"/></signal>
</interface></node>`;
let state = {state: 'running', message: 'Teste sintético', generation: 1, revision: 1,
    translation: 'A porta está trancada. Encontre a chave.',
    region: {x: 120, y: 500, width: 1040, height: 140}, monitor: {x: 0, y: 0, width: 1280, height: 720}, frame_size: [1280, 720]};
let object;
let clicks = 0;
const setText = text => {
    state.translation = text; state.revision++;
    object.emit_signal('TranslationChanged', new GLib.Variant('(tts)', [state.generation, state.revision, text]));
};
const update = value => {
    state.state = value; state.generation++;
    object.emit_signal('StatusChanged', new GLib.Variant('(s)', [JSON.stringify(state)]));
};
object = Gio.DBusExportedObject.wrapJSObject(xml, {
    GetStatus: () => JSON.stringify(state), Stop: () => update('idle'),
    Pause: () => update('paused'), Resume: () => update('running'),
    GetClicks: () => clicks,
    LongSubtitle: () => setText('Esta é uma tradução longa para verificar se a legenda cresce automaticamente e continua fora da área de reconhecimento. '.repeat(7)),
    ShortSubtitle: () => setText('Tradução curta.'),
    WindowSource: () => {
        state.source_type = 'window';
        state.region = {x: 0, y: 0, width: 640, height: 480};
        state.frame_size = [640, 480];
        update('running');
    },
});
object.export(Gio.DBus.session, '/io/github/areatranslator/Service');
Gio.bus_own_name_on_connection(Gio.DBus.session, name, Gio.BusNameOwnerFlags.NONE, null, null);
const app = new Gtk.Application({application_id: 'io.github.areatranslator.TestGame'});
app.connect('activate', () => {
    const window = new Gtk.ApplicationWindow({application: app, title: 'Area Translator — Synthetic Game'});
    const area = new Gtk.DrawingArea();
    area.set_draw_func((_area, cr, width, height) => {
        cr.setSourceRGB(0.05, 0.1, 0.14); cr.paint();
        cr.setSourceRGB(0.1, 0.2, 0.25); cr.rectangle(60, 60, width - 120, height - 120); cr.fill();
        cr.setSourceRGB(0.85, 0.9, 0.93); cr.setFontSize(26);
        cr.moveTo(120, 140); cr.showText('FULLSCREEN SYNTHETIC TEST');
        cr.setSourceRGB(0.01, 0.03, 0.06); cr.rectangle(120, 500, 1040, 140); cr.fill();
        cr.setSourceRGB(1, 1, 1); cr.setFontSize(30);
        cr.moveTo(145, 560); cr.showText('The door is locked. Find the key.');
    });
    const click = new Gtk.GestureClick();
    click.connect('pressed', () => { clicks++; print('Synthetic app received click'); });
    click.set_button(0);
    click.set_propagation_phase(Gtk.PropagationPhase.CAPTURE);
    area.add_controller(click);
    const motion = new Gtk.EventControllerMotion();
    motion.connect('enter', () => print('Synthetic app received pointer enter'));
    area.add_controller(motion);
    window.set_child(area); window.fullscreen(); window.present();
});
app.run([]);
