use std::f32::consts::TAU;

use super::settings::AppSettings;

pub struct RackControls {
    gain: f32,
    tone: f32,
    level: f32,
}

impl RackControls {
    pub fn new() -> Self {
        Self {
            gain: 0.5,
            tone: 0.5,
            level: 0.5,
        }
    }

    pub fn from_settings(settings: &AppSettings) -> Self {
        Self {
            gain: settings.gain,
            tone: settings.tone,
            level: settings.level,
        }
    }

    pub fn gain(&self) -> f32 {
        self.gain
    }

    pub fn tone(&self) -> f32 {
        self.tone
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Overdrive Module");
        ui.add_space(20.0);

        ui.horizontal(|ui| {
            for (label, value) in [
                ("Gain", &mut self.gain),
                ("Tone", &mut self.tone),
                ("Level", &mut self.level),
            ] {
                ui.allocate_ui_with_layout(
                    egui::vec2(50.0, 80.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let response = knob(ui, value);
                        ui.label(label);
                        if response.changed() {
                            println!("{label} updated to: {value:.2}");
                        }
                    },
                );
                ui.add_space(20.0);
            }
        });
    }
}

fn knob(ui: &mut egui::Ui, value: &mut f32) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(50.0, 50.0), egui::Sense::drag());

    if response.dragged() {
        *value = (*value - response.drag_delta().y * 0.005).clamp(0.0, 1.0);
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let center = rect.center();
        let radius = rect.width() / 2.0;
        ui.painter().circle_filled(center, radius, visuals.bg_fill);
        ui.painter()
            .circle_stroke(center, radius, visuals.bg_stroke);

        let angle = egui::lerp(-TAU * 0.375..=TAU * 0.375, *value);
        let direction = egui::vec2(angle.sin(), -angle.cos());
        ui.painter().line_segment(
            [
                center + direction * 10.0,
                center + direction * (radius - 4.0),
            ],
            visuals.fg_stroke,
        );
    }

    response
}
