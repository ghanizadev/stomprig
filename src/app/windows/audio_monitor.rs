use crate::app::audio::{AudioEngine, SignalLevels};
use std::time::Duration;

const MAX_WAVEFORM_SAMPLES: usize = 1024;
const WAVEFORM_HEIGHT: f32 = 48.0;
const WAVEFORM_AMPLITUDE: f32 = 0.75;
const WAVEFORM_SAMPLE_STRIDE: usize = 2;

pub struct AudioMonitor {
    samples: Vec<f32>,
    levels: SignalLevels,
}

impl Default for AudioMonitor {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            levels: SignalLevels::default(),
        }
    }
}

impl AudioMonitor {
    pub fn idle_ui(&self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Audio monitor");
            ui.weak("Audio engine is not running");
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &mut AudioEngine) -> Option<String> {
        engine.drain_output(&mut self.samples);
        if self.samples.len() > MAX_WAVEFORM_SAMPLES {
            self.samples
                .drain(..self.samples.len() - MAX_WAVEFORM_SAMPLES);
        }
        self.levels = engine.levels();
        let error = engine.take_error();
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Audio monitor");
            meter(ui, "Input peak", self.levels.input_peak);
            meter(ui, "Input RMS", self.levels.input_rms);
            meter(ui, "Output peak", self.levels.output_peak);
            meter(ui, "Output RMS", self.levels.output_rms);
            ui.label(format!("{} samples", self.samples.len()));
        });
        waveform(ui, &self.samples);
        ui.ctx().request_repaint_after(Duration::from_millis(33));
        error
    }
}

fn meter(ui: &mut egui::Ui, label: &str, level: f32) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::ProgressBar::new(level.clamp(0.0, 1.0)).desired_width(80.0));
        ui.label(format!("{:.1} dB", level.max(0.000_001).log10() * 20.0));
    });
}
fn waveform(ui: &mut egui::Ui, samples: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), WAVEFORM_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let center = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left(), center),
            egui::pos2(rect.right(), center),
        ],
        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color()),
    );
    if samples.len() < 2 {
        return;
    }
    let point_count = samples.len().div_ceil(WAVEFORM_SAMPLE_STRIDE);
    let x_denominator = point_count.saturating_sub(1).max(1) as f32;
    let points = samples
        .iter()
        .step_by(WAVEFORM_SAMPLE_STRIDE)
        .enumerate()
        .map(|(index, sample)| {
            let x = rect.left() + index as f32 / x_denominator * rect.width();
            egui::pos2(
                x,
                center - sample.clamp(-1.0, 1.0) * rect.height() * WAVEFORM_AMPLITUDE,
            )
        })
        .collect::<Vec<_>>();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.0_f32, egui::Color32::LIGHT_BLUE),
    ));
}
