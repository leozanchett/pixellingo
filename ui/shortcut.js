import Gtk from 'gi://Gtk?version=4.0';
import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export const SHORTCUT_PATH = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/pixellingo-refresh/';
export const DEFAULT_SHORTCUT = '<Super><Shift>r';
const ROOT_SCHEMA = 'org.gnome.settings-daemon.plugins.media-keys';
const CUSTOM_SCHEMA = `${ROOT_SCHEMA}.custom-keybinding`;
const custom = path => new Gio.Settings({schema_id: CUSTOM_SCHEMA, path});
const modifiers = new Set(['Shift_L', 'Shift_R', 'Control_L', 'Control_R', 'Alt_L', 'Alt_R',
    'Super_L', 'Super_R', 'Meta_L', 'Meta_R', 'ISO_Level3_Shift', 'Caps_Lock', 'Num_Lock']);

export function acceleratorFromKey(keyval, state) {
    if (modifiers.has(Gdk.keyval_name(keyval))) return null;
    const key = Gdk.keyval_to_lower(keyval);
    const mods = state & Gtk.accelerator_get_default_mod_mask();
    const strongModifier = mods & (Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.ALT_MASK | Gdk.ModifierType.SUPER_MASK);
    const functionKey = key >= Gdk.KEY_F1 && key <= Gdk.KEY_F35;
    if (!Gtk.accelerator_valid(key, mods) || (!strongModifier && !functionKey))
        throw new Error('Use Ctrl, Alt ou Super com uma tecla, ou uma tecla de função como F8.');
    return Gtk.accelerator_name(key, mods);
}

function canonical(value) {
    const [ok, key, mods] = Gtk.accelerator_parse(value);
    return ok && key ? Gtk.accelerator_name(Gdk.keyval_to_lower(key), mods) : null;
}

export function readShortcut() {
    return custom(SHORTCUT_PATH).get_string('binding');
}

export function shortcutLabel(value = readShortcut()) {
    const [ok, key, mods] = Gtk.accelerator_parse(value);
    return ok && key ? Gtk.accelerator_get_label(key, mods) : 'Desativado';
}

export function shortcutConflict(value) {
    if (!value) return null;
    const target = canonical(value);
    const root = new Gio.Settings({schema_id: ROOT_SCHEMA});
    for (const path of root.get_strv('custom-keybindings')) {
        if (path === SHORTCUT_PATH) continue;
        const settings = custom(path);
        if (canonical(settings.get_string('binding')) === target)
            return settings.get_string('name') || 'outro atalho personalizado';
    }
    let source = Gio.SettingsSchemaSource.get_default();
    const extensionSchemas = GLib.build_filenamev([GLib.get_user_data_dir(), 'gnome-shell',
        'extensions', 'area-translator@local', 'schemas']);
    if (GLib.file_test(GLib.build_filenamev([extensionSchemas, 'gschemas.compiled']), GLib.FileTest.EXISTS))
        source = Gio.SettingsSchemaSource.new_from_directory(extensionSchemas, source, false);
    for (const id of ['org.gnome.desktop.wm.keybindings', 'org.gnome.mutter.keybindings',
        'org.gnome.mutter.wayland.keybindings', 'org.gnome.shell.keybindings', ROOT_SCHEMA,
        'org.gnome.shell.extensions.area-translator']) {
        const schema = source.lookup(id, true);
        if (!schema) continue;
        const settings = new Gio.Settings({settings_schema: schema});
        for (const key of schema.list_keys()) {
            const variant = settings.get_value(key);
            const type = variant.get_type_string();
            const values = type === 'as' ? variant.deepUnpack() : type === 's' ? [variant.deepUnpack()] : [];
            if (values.some(binding => canonical(binding) === target))
                return id === 'org.gnome.shell.extensions.area-translator' ? 'Pausar / retomar tradução' : `Ubuntu: ${key}`;
        }
    }
    return null;
}

export function saveShortcut(value) {
    let binding = '';
    if (value) {
        const [ok, key, mods] = Gtk.accelerator_parse(value);
        if (!ok || !(binding = acceleratorFromKey(key, mods))) throw new Error('Combinação inválida.');
        const conflict = shortcutConflict(binding);
        if (conflict) throw new Error(`Essa combinação já está em uso por “${conflict}”. Escolha outra.`);
    }
    const root = new Gio.Settings({schema_id: ROOT_SCHEMA});
    if (!root.get_strv('custom-keybindings').includes(SHORTCUT_PATH))
        throw new Error('O atalho não está instalado. Execute novamente scripts/install.sh.');
    if (!custom(SHORTCUT_PATH).set_string('binding', binding))
        throw new Error('O Ubuntu não permitiu alterar esse atalho.');
    Gio.Settings.sync();
}

export function showShortcutDialog(parent, onSaved) {
    const dialog = new Gtk.Window({title: 'Atalho de tradução', transient_for: parent, modal: true,
        default_width: 490, resizable: false});
    const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 16,
        margin_start: 24, margin_end: 24, margin_top: 24, margin_bottom: 24});
    dialog.set_child(box);
    let candidate = readShortcut();
    let recording = true;
    let surface;
    const help = new Gtk.Label({label: 'Pressione a combinação desejada. Esc cancela.', wrap: true, xalign: 0});
    const preview = new Gtk.Label({label: shortcutLabel(candidate)});
    preview.add_css_class('title-2');
    const error = new Gtk.Label({wrap: true, xalign: 0, max_width_chars: 50});
    error.add_css_class('error');
    box.append(help); box.append(preview); box.append(error);
    const action = (label, callback) => {
        const button = new Gtk.Button({label}); button.connect('clicked', callback); return button;
    };
    const choose = value => {
        candidate = value; recording = false; preview.label = shortcutLabel(value);
        help.label = 'Clique em Salvar para aplicar a alteração.'; error.label = '';
    };
    const options = new Gtk.Box({spacing: 8});
    options.append(action('Gravar novamente', () => {
        recording = true; error.label = ''; help.label = 'Pressione a combinação desejada. Esc cancela.';
    }));
    options.append(action('Restaurar padrão', () => choose(DEFAULT_SHORTCUT)));
    options.append(action('Desativar', () => choose('')));
    box.append(options);
    const buttons = new Gtk.Box({spacing: 8, halign: Gtk.Align.END});
    buttons.append(action('Cancelar', () => dialog.close()));
    const save = action('Salvar', () => {
        try { saveShortcut(candidate); onSaved(); dialog.close(); }
        catch (e) { error.label = e.message; }
    });
    save.add_css_class('suggested-action'); buttons.append(save); box.append(buttons);
    const controller = new Gtk.EventControllerKey({propagation_phase: Gtk.PropagationPhase.CAPTURE});
    controller.set_name('pixellingo-shortcut-recorder');
    controller.connect('key-pressed', (_controller, key, _code, state) => {
        if (key === Gdk.KEY_Escape) { dialog.close(); return true; }
        if (!recording) return false;
        try {
            const value = acceleratorFromKey(key, state);
            if (value) choose(value);
        } catch (e) { error.label = e.message; }
        return true;
    });
    dialog.add_controller(controller);
    dialog.connect('map', () => {
        surface = dialog.get_surface();
        surface.inhibit_system_shortcuts(null);
    });
    dialog.connect('unmap', () => { surface?.restore_system_shortcuts(); surface = null; });
    dialog.present();
    return dialog;
}
