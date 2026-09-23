#!/usr/bin/gjs -m
import Gtk from 'gi://Gtk?version=4.0';
import Gdk from 'gi://Gdk?version=4.0';
import GdkPixbuf from 'gi://GdkPixbuf';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Secret from 'gi://Secret';

const BUS = 'io.github.areatranslator.Service';
const PATH = '/io/github/areatranslator/Service';
const OVERLAY = '/io/github/areatranslator/Overlay';
const schema = new Secret.Schema('io.github.areatranslator.Credential', Secret.SchemaFlags.NONE,
    {application: Secret.SchemaAttributeType.STRING});
const attributes = {application: 'area-translator'};

function call(destination, path, iface, method, signature = null, values = []) {
    return new Promise((resolve, reject) => Gio.DBus.session.call(destination, path, iface, method,
        signature ? new GLib.Variant(signature, values) : null, null, Gio.DBusCallFlags.NONE,
        15000, null, (connection, result) => {
            try { resolve(connection.call_finish(result).deepUnpack()); }
            catch (error) { reject(error); }
        }));
}
const service = (method, signature, values) => call(BUS, PATH, BUS, method, signature, values);
const sleep = ms => new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => {
    resolve(); return GLib.SOURCE_REMOVE;
}));
const lookup = () => new Promise((resolve, reject) => Secret.password_lookup(schema, attributes, null,
    (_source, result) => { try { resolve(Secret.password_lookup_finish(result)); } catch (e) { reject(e); } }));
const store = password => new Promise((resolve, reject) => Secret.password_store(schema, attributes,
    Secret.COLLECTION_DEFAULT, 'Area Translator — Google Cloud', password, null,
    (_source, result) => { try { resolve(Secret.password_store_finish(result)); } catch (e) { reject(e); } }));

const app = new Gtk.Application({application_id: 'io.github.areatranslator.App'});
const smokeTest = ARGV.includes('--smoke-test');
const smokeSelection = ARGV.includes('--smoke-selection');
let window;
let selecting = false;
let selectionCancelled = false;
let credentialsReady = false;
let message;

function showError(error) {
    message.label = String(error.message ?? error).replace(/^GDBus\.Error:[^:]+:\s*/, '');
    window.present();
}

function button(label, action, primary = false) {
    const result = new Gtk.Button({label});
    if (primary) result.add_css_class('suggested-action');
    result.connect('clicked', () => Promise.resolve().then(action).catch(showError));
    return result;
}

function page(title, subtitle) {
    const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 16,
        margin_start: 24, margin_end: 24, margin_top: 24, margin_bottom: 24});
    const heading = new Gtk.Label({label: title, xalign: 0});
    heading.add_css_class('title-1');
    box.append(heading);
    box.append(new Gtk.Label({label: subtitle, wrap: true, xalign: 0}));
    message = new Gtk.Label({label: '', wrap: true, xalign: 0, selectable: true});
    message.add_css_class('error');
    window.set_child(box);
    return box;
}

function settings() {
    window.set_default_size(560, 300);
    const box = page('Tradutor de área', 'Inglês → português brasileiro, sobre qualquer aplicativo.');
    const entry = new Gtk.PasswordEntry({placeholder_text: credentialsReady
        ? 'Chave salva no chaveiro — preencha apenas para trocar' : 'Chave da Cloud Translation API', show_peek_icon: true});
    box.append(entry);
    box.append(new Gtk.Label({label: 'As imagens ficam neste computador. Somente o texto é enviado ao Google Cloud. O serviço pode cobrar pelo uso.', wrap: true, xalign: 0}));
    const actions = new Gtk.Box({spacing: 8});
    actions.append(button('Salvar chave', async () => {
        const key = entry.text.trim();
        await service('SetApiKey', '(s)', [key]);
        await store(key);
        entry.text = '';
        credentialsReady = true;
        message.label = 'Chave salva no chaveiro do sistema.';
    }));
    actions.append(button('Selecionar área', async () => {
        if (!credentialsReady) throw new Error('Salve a chave do Google Cloud primeiro.');
        await selectArea();
    }, true));
    box.append(actions);
    box.append(message);
    box.append(new Gtk.Label({label: 'Depois de iniciar, use o ícone “Tradutor de área” na barra superior para pausar, reposicionar a legenda ou encerrar.', wrap: true, xalign: 0}));
}

async function selectArea() {
    let monitors;
    try {
        const [json] = await call('org.gnome.Shell', OVERLAY, 'io.github.areatranslator.Overlay', 'GetMonitors');
        monitors = JSON.parse(json);
    } catch (_) {
        throw new Error('Ative a extensão “Tradutor de área” no GNOME. Após a primeira instalação, pode ser necessário sair da sessão e entrar novamente.');
    }
    if (!monitors.length) throw new Error('Nenhum monitor disponível.');
    selecting = true;
    selectionCancelled = false;
    window.hide();
    try {
        await service('BeginSelection');
        let bytes = null;
        let status;
        for (let attempt = 0; attempt < 600 && !selectionCancelled; attempt++) {
            await sleep(250);
            const [json] = await service('GetStatus');
            status = JSON.parse(json);
            if (status.state === 'error' || status.state === 'idle') throw new Error(status.message);
            if (status.state === 'selecting') {
                try { [bytes] = await service('GetPreview'); break; } catch (_) { /* first frame pending */ }
            }
        }
        if (!bytes) throw new Error('Seleção cancelada ou captura sem resposta. Tente novamente.');
        renderSelection(bytes, status, monitors);
        window.present();
    } catch (error) {
        selecting = false;
        await service('Stop').catch(() => {});
        settings();
        throw error;
    }
}

function renderSelection(bytes, status, monitors) {
    window.set_default_size(960, 680);
    const loader = new GdkPixbuf.PixbufLoader();
    loader.write(bytes);
    loader.close();
    const pixbuf = loader.get_pixbuf();
    const width = pixbuf.width;
    const height = pixbuf.height;
    const box = page('Selecione a região de texto', 'Arraste sobre a prévia. Deixe espaço acima ou abaixo para a legenda. A área fica fixa na tela.');
    const monitorNames = monitors.map((m, i) => `${i + 1}. ${m.name} — ${m.width} × ${m.height}`);
    const dropdown = Gtk.DropDown.new_from_strings(monitorNames);
    const match = monitors.findIndex(m => status.portal_position && m.x === status.portal_position[0]
        && m.y === status.portal_position[1]);
    dropdown.selected = match >= 0 ? match : monitors.length === 1 ? 0 : Gtk.INVALID_LIST_POSITION;
    box.append(new Gtk.Label({label: 'Monitor compartilhado (selecione o mesmo escolhido no diálogo do sistema):', xalign: 0, wrap: true}));
    box.append(dropdown);
    const drawing = new Gtk.DrawingArea({hexpand: true, vexpand: true, content_width: 800, content_height: 420});
    let region = null;
    let start = null;
    let transform = {scale: 1, x: 0, y: 0};
    drawing.set_draw_func((_area, cr, availableWidth, availableHeight) => {
        const scale = Math.min(availableWidth / width, availableHeight / height);
        const x = (availableWidth - width * scale) / 2;
        const y = (availableHeight - height * scale) / 2;
        transform = {scale, x, y};
        cr.setSourceRGB(0.07, 0.08, 0.1); cr.paint();
        cr.save(); cr.translate(x, y); cr.scale(scale, scale);
        Gdk.cairo_set_source_pixbuf(cr, pixbuf, 0, 0); cr.paint();
        if (region) {
            cr.rectangle(region.x, region.y, region.width, region.height);
            cr.setSourceRGBA(0.2, 0.65, 1, 0.15); cr.fillPreserve();
            cr.setSourceRGB(0.2, 0.75, 1); cr.setLineWidth(2 / scale); cr.stroke();
        }
        cr.restore();
    });
    const point = (x, y) => ({x: Math.max(0, Math.min(width, (x - transform.x) / transform.scale)),
        y: Math.max(0, Math.min(height, (y - transform.y) / transform.scale))});
    const drag = new Gtk.GestureDrag();
    let screenStart;
    drag.connect('drag-begin', (_gesture, x, y) => { screenStart = {x, y}; start = point(x, y); });
    drag.connect('drag-update', (_gesture, dx, dy) => {
        if (!start) return;
        const end = point(screenStart.x + dx, screenStart.y + dy);
        region = {x: Math.floor(Math.min(start.x, end.x)), y: Math.floor(Math.min(start.y, end.y)),
            width: Math.floor(Math.abs(end.x - start.x)), height: Math.floor(Math.abs(end.y - start.y))};
        drawing.queue_draw();
    });
    drawing.add_controller(drag);
    box.append(drawing);
    const actions = new Gtk.Box({spacing: 8});
    actions.append(button('Cancelar', async () => {
        selectionCancelled = true; selecting = false;
        await service('Stop'); settings();
    }));
    actions.append(button('Iniciar tradução', async () => {
        if (!region || region.width < 16 || region.height < 16) throw new Error('Arraste para marcar uma área de texto.');
        const monitor = monitors[dropdown.selected];
        if (!monitor) throw new Error('Selecione o monitor que está sendo compartilhado.');
        // Compare aspect ratios: logical and pixel coordinates can have different scales.
        if (Math.abs((width / height) / (monitor.width / monitor.height) - 1) > 0.03)
            throw new Error('O monitor escolhido não corresponde ao formato da captura.');
        await service('SetRegion', '(ss)', [JSON.stringify(region), JSON.stringify(monitor)]);
        selecting = false;
        window.close();
    }, true));
    box.append(actions);
    box.append(message);
}

app.connect('activate', () => {
    if (window) { window.present(); return; }
    window = new Gtk.ApplicationWindow({application: app, title: 'Tradutor de área', default_width: 560, default_height: 300});
    window.connect('close-request', () => {
        if (selecting) {
            selectionCancelled = true;
            service('Stop').finally(() => app.quit());
            return true;
        }
        return false;
    });
    settings();
    window.present();
    if (smokeTest || smokeSelection) {
        if (smokeSelection) {
            const [, bytes] = Gio.File.new_for_path(GLib.getenv('AREA_TRANSLATOR_TEST_IMAGE')).load_contents(null);
            renderSelection(bytes, {portal_position: [0, 0]}, [{x: 0, y: 0, width: 1280, height: 720, name: 'Monitor de teste'}]);
        }
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => {
            print('GTK4 settings window rendered.'); app.quit(); return GLib.SOURCE_REMOVE;
        });
        return;
    }
    lookup().then(async key => {
        if (key) { await service('SetApiKey', '(s)', [key]); credentialsReady = true; settings(); }
    }).catch(showError);
});
app.run(ARGV.filter(arg => !['--smoke-test', '--smoke-selection'].includes(arg)));
