use super::{LivePalette, LiveTunerData};

pub(super) fn show(ui: &mut egui::Ui, palette: &LivePalette, data: &LiveTunerData) {
    let note = (!data.bypassed).then(|| tuner_note(data)).flatten();
    let has_note = note.is_some();
    let note_text = if data.bypassed {
        "TUNER BYPASSED"
    } else if has_note {
        note.as_deref().unwrap_or("—")
    } else {
        "READY"
    };
    let note_size = if data.bypassed || !has_note {
        84.0
    } else {
        112.0
    };
    let note_color = if data.bypassed {
        palette.amber
    } else if has_note {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    ui.vertical_centered(|ui| {
        show_fixed_text(
            ui,
            130.0,
            egui::RichText::new(note_text)
                .size(note_size)
                .strong()
                .color(note_color),
        );
        show_fixed_text(
            ui,
            50.0,
            egui::RichText::new(if has_note {
                format_cents(data.readout.cents)
            } else {
                "—".to_owned()
            })
            .size(38.0)
            .strong()
            .color(if data.bypassed {
                palette.amber
            } else {
                tuning_color(data.readout.cents, palette)
            }),
        );
        show_fixed_text(
            ui,
            36.0,
            egui::RichText::new(if has_note {
                format!("{:.1} Hz", data.readout.frequency_hz)
            } else {
                "—".to_owned()
            })
            .size(26.0)
            .strong()
            .color(palette.text_secondary),
        );
        ui.add_space(22.0);
        show_tuning_meter(ui, palette, data.readout.cents, has_note);
    });
}

fn show_fixed_text(ui: &mut egui::Ui, height: f32, text: egui::RichText) {
    let width = ui.available_width().max(1.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            ui.label(text);
        },
    );
}

fn show_tuning_meter(ui: &mut egui::Ui, palette: &LivePalette, cents: f32, has_note: bool) {
    const BAR_COUNT: usize = 61;
    const HEIGHT: f32 = 84.0;
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::hover());
    let painter = ui.painter();
    let gap = 3.0;
    let bar_width = ((rect.width() - gap * (BAR_COUNT - 1) as f32) / BAR_COUNT as f32).max(1.0);
    let center = BAR_COUNT / 2;
    let target = if has_note {
        Some(((cents.clamp(-50.0, 50.0) + 50.0) / 100.0 * (BAR_COUNT - 1) as f32).round() as usize)
    } else {
        None
    };

    for index in 0..BAR_COUNT {
        let distance = index.abs_diff(center);
        let base_color = if distance == 0 {
            palette.green
        } else if distance < 4 {
            palette.amber
        } else {
            palette.cyan
        };
        let active = target == Some(index);
        let color = if active {
            base_color
        } else {
            base_color.gamma_multiply(if has_note { 0.24 } else { 0.10 })
        };
        let bar_height = if distance == 0 { HEIGHT } else { HEIGHT * 0.62 };
        let left = rect.left() + index as f32 * (bar_width + gap);
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(left, rect.center().y - bar_height * 0.5),
                egui::vec2(bar_width, bar_height),
            ),
            2.0,
            color,
        );
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("FLAT")
                .size(16.0)
                .strong()
                .color(palette.cyan),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new("SHARP")
                    .size(16.0)
                    .strong()
                    .color(palette.cyan),
            );
        });
    });
}

fn tuner_note(data: &LiveTunerData) -> Option<String> {
    if data.readout.confidence < 0.25
        || data.readout.frequency_hz <= 0.0
        || !data.a4_pitch_hz.is_finite()
        || data.a4_pitch_hz <= 0.0
    {
        return None;
    }
    const NOTE_NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    let midi = (69.0
        + 12.0 * (data.readout.frequency_hz / data.a4_pitch_hz).log2()
        + data.transposition_semitones as f32)
        .round() as i32;
    Some(format!(
        "{}{}",
        NOTE_NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    ))
}

fn format_cents(cents: f32) -> String {
    if cents.abs() < 0.05 {
        "IN TUNE".to_owned()
    } else {
        format!("{cents:+.1} CENTS")
    }
}

fn tuning_color(cents: f32, palette: &LivePalette) -> egui::Color32 {
    if cents.abs() < 5.0 {
        palette.green
    } else if cents.abs() < 20.0 {
        palette.amber
    } else {
        palette.red
    }
}
