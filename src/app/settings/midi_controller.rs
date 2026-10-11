use super::super::midi::MidiControlMode;
use super::super::ui::{
    Button as UiButton, ButtonType, Select, SelectItem, Theme, caption, section, square_button,
};
use lucide_icons::Icon;
use vst3_host::midi_input::MidiInputPort;

const CONTROLLER_MODE_WIDTH: f32 = 150.0;
const ACTION_COLUMN_WIDTH: f32 = 44.0;

pub(crate) struct MidiEffectSettingsRow {
    pub(crate) name: String,
    pub(crate) mapping_label: Option<String>,
    pub(crate) control_mode: MidiControlMode,
}

pub(super) struct MidiControllerSettings<'a> {
    pub(super) devices: &'a [MidiInputPort],
    pub(super) input_device: &'a mut Option<String>,
    pub(super) enabled: &'a mut bool,
    pub(super) default_mode: &'a mut MidiControlMode,
    pub(super) effects: &'a mut [MidiEffectSettingsRow],
    pub(super) learning_plugin: Option<usize>,
}

#[derive(Default)]
pub(super) struct MidiControllerActions {
    pub(super) refresh_devices: bool,
    pub(super) mappings_changed: bool,
    pub(super) learn_plugin: Option<usize>,
    pub(super) remove_plugin: Option<usize>,
}

pub(super) fn show(
    ui: &mut egui::Ui,
    theme: &Theme,
    settings: MidiControllerSettings<'_>,
) -> MidiControllerActions {
    let MidiControllerSettings {
        devices,
        input_device,
        enabled,
        default_mode,
        effects,
        learning_plugin,
    } = settings;
    let mut actions = MidiControllerActions::default();
    ui.vertical(|ui| {
        ui.group(|ui| {
            section(ui, "MIDI input");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Device");
                let mut selected = input_device.clone();
                let items = devices
                    .iter()
                    .map(|device| SelectItem::option(device.name(), device.name()))
                    .collect::<Vec<_>>();
                if Select::new("midi-input-device", &mut selected, &items)
                    .placeholder("No MIDI input")
                    .width(ui.available_width().min(300.0))
                    .show(ui, theme)
                    .changed()
                {
                    *input_device = selected;
                }
                if UiButton::new("Refresh")
                    .button_type(ButtonType::Outline)
                    .show(ui, theme)
                    .clicked()
                {
                    actions.refresh_devices = true;
                }
            });
            ui.checkbox(enabled, "Receive MIDI controller input");
            caption(
                ui,
                egui::RichText::new(
                    "MIDI mappings are saved with presets; the selected input and default controller behavior are saved here.",
                ),
            );
            caption(
                ui,
                egui::RichText::new(
                    "Program Change selects the board at that position in the selected live preset. PC 0 selects the first board.",
                ),
            );
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            section(ui, "Controller");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Default switch type");
                if select_control_mode(
                    ui,
                    theme,
                    "midi-default-controller-mode",
                    default_mode,
                    true,
                    CONTROLLER_MODE_WIDTH,
                ) {
                    for effect in effects.iter_mut() {
                        if effect.mapping_label.is_some() {
                            actions.mappings_changed = true;
                        }
                        effect.control_mode = *default_mode;
                    }
                }
            });
            caption(
                ui,
                egui::RichText::new(
                    "Changing the default updates assigned controls and sets the behavior used by future mappings. For bypass, Value maps 0–63 to bypass off and 64–127 to bypass on.",
                ),
            );
            ui.add_space(6.0);

            let column_spacing = ui.spacing().item_spacing.x;
            let effect_column_width = (ui.available_width()
                - CONTROLLER_MODE_WIDTH
                - ACTION_COLUMN_WIDTH * 2.0
                - column_spacing * 3.0)
                .max(120.0);
            egui::Grid::new("midi-controller-effects")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Effect");
                    ui.strong("Trigger");
                    ui.add_sized(
                        [ACTION_COLUMN_WIDTH, ui.spacing().interact_size.y],
                        egui::Label::new(egui::RichText::new("Learn").strong()),
                    );
                    ui.add_sized(
                        [ACTION_COLUMN_WIDTH, ui.spacing().interact_size.y],
                        egui::Label::new(egui::RichText::new("Remove").strong()),
                    );
                    ui.end_row();

                    for (index, effect) in effects.iter_mut().enumerate() {
                        let mapped = effect.mapping_label.is_some();
                        ui.allocate_ui_with_layout(
                            egui::vec2(effect_column_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(effect_column_width);
                                ui.add(egui::Label::new(&effect.name).wrap());
                                if let Some(mapping_label) = effect.mapping_label.as_deref() {
                                    caption(ui, egui::RichText::new(mapping_label));
                                }
                            },
                        );
                        let mode_changed = select_control_mode(
                            ui,
                            theme,
                            ("midi-effect-controller-mode", index),
                            &mut effect.control_mode,
                            mapped,
                            CONTROLLER_MODE_WIDTH,
                        );
                        actions.mappings_changed |= mode_changed;

                        let learning = learning_plugin == Some(index);
                        let learn_clicked = ui
                            .allocate_ui_with_layout(
                                egui::vec2(ACTION_COLUMN_WIDTH, 32.0),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    square_button(
                                        ui,
                                        theme,
                                        Icon::AudioLines,
                                        true,
                                        if learning {
                                            "Listening for MIDI input"
                                        } else {
                                            "Learn MIDI control"
                                        },
                                    )
                                },
                            )
                            .inner
                            .clicked();
                        if learn_clicked {
                            actions.learn_plugin = Some(index);
                        }
                        let remove_clicked = ui
                            .allocate_ui_with_layout(
                                egui::vec2(ACTION_COLUMN_WIDTH, 32.0),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    square_button(
                                        ui,
                                        theme,
                                        Icon::Trash2,
                                        mapped,
                                        "Remove MIDI mapping",
                                    )
                                },
                            )
                            .inner
                            .clicked();
                        if remove_clicked {
                            actions.remove_plugin = Some(index);
                        }
                        ui.end_row();
                    }
                });
        });
    });
    actions
}

fn select_control_mode<Id: std::hash::Hash + std::fmt::Debug>(
    ui: &mut egui::Ui,
    theme: &Theme,
    id: Id,
    mode: &mut MidiControlMode,
    enabled: bool,
    width: f32,
) -> bool {
    let mut selected = Some(mode.key().to_owned());
    let items = MidiControlMode::ALL
        .into_iter()
        .map(|mode| SelectItem::option(mode.key(), mode.label()))
        .collect::<Vec<_>>();
    let _ = Select::new(id, &mut selected, &items)
        .width(width)
        .enabled(enabled)
        .show(ui, theme);
    if let Some(selected_mode) = selected.as_deref().and_then(MidiControlMode::from_key)
        && selected_mode != *mode
    {
        *mode = selected_mode;
        true
    } else {
        false
    }
}
