use crate::app::audio::{AudioEngine, SignalLevels};
use std::collections::VecDeque;
use std::time::Duration;

const MAX_WAVEFORM_SAMPLES: usize = 1024;
const WAVEFORM_HEIGHT: f32 = 48.0;
const WAVEFORM_AMPLITUDE: f32 = 0.75;
const WAVEFORM_SAMPLE_STRIDE: usize = 2;
const METER_WIDTH: f32 = 120.0;
const METER_HEIGHT: f32 = 12.0;
const METER_SEGMENT_WIDTH: f32 = 4.0;
const METER_SEGMENT_GAP: f32 = 1.0;
const METER_SEGMENT_COUNT: usize =
    ((METER_WIDTH + METER_SEGMENT_GAP) / (METER_SEGMENT_WIDTH + METER_SEGMENT_GAP)) as usize;
const METER_CLIP_SEGMENTS: usize = 3;
const METER_SMOOTHING_SAMPLES: usize = 6;

pub struct AudioMonitor {
    samples: Vec<f32>,
    levels: SignalLevels,
    input_peak: SmoothedLevel,
    input_rms: SmoothedLevel,
    output_peak: SmoothedLevel,
    output_rms: SmoothedLevel,
    input_peak_clip: ClipState,
    input_rms_clip: ClipState,
    output_peak_clip: ClipState,
    output_rms_clip: ClipState,
}

impl Default for AudioMonitor {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            levels: SignalLevels::default(),
            input_peak: SmoothedLevel::default(),
            input_rms: SmoothedLevel::default(),
            output_peak: SmoothedLevel::default(),
            output_rms: SmoothedLevel::default(),
            input_peak_clip: ClipState::default(),
            input_rms_clip: ClipState::default(),
            output_peak_clip: ClipState::default(),
            output_rms_clip: ClipState::default(),
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
        let underruns = engine.take_output_underruns();
        let input_peak = self.input_peak.update(self.levels.input_peak);
        let input_rms = self.input_rms.update(self.levels.input_rms);
        let output_peak = self.output_peak.update(self.levels.output_peak);
        let output_rms = self.output_rms.update(self.levels.output_rms);
        self.input_peak_clip.observe(self.levels.input_peak);
        self.input_rms_clip.observe(self.levels.input_rms);
        self.output_peak_clip.observe(self.levels.output_peak);
        self.output_rms_clip.observe(self.levels.output_rms);
        let input_peak_clipping = self.input_peak_clip.signal_clipping;
        let input_rms_clipping = self.input_rms_clip.signal_clipping;
        let output_peak_clipping = self.output_peak_clip.signal_clipping;
        let output_rms_clipping = self.output_rms_clip.signal_clipping;
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Audio monitor");
            meter(
                ui,
                "Input peak",
                input_peak,
                input_peak_clipping,
                &mut self.input_peak_clip,
            );
            meter(
                ui,
                "Input RMS",
                input_rms,
                input_rms_clipping,
                &mut self.input_rms_clip,
            );
            meter(
                ui,
                "Output peak",
                output_peak,
                output_peak_clipping,
                &mut self.output_peak_clip,
            );
            meter(
                ui,
                "Output RMS",
                output_rms,
                output_rms_clipping,
                &mut self.output_rms_clip,
            );
            ui.label(format!("{} samples", self.samples.len()));
            if underruns > 0 {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("{underruns} audio underrun(s)"),
                );
            }
        });
        waveform(ui, &self.samples);
        ui.ctx().request_repaint_after(Duration::from_millis(33));
        error
    }
}

fn meter(ui: &mut egui::Ui, label: &str, level: f32, clipping: bool, clip_state: &mut ClipState) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(SegmentedMeter::new(level, clipping));
        ui.add_space(2.0);
        ui.add(ClipIndicator::new(clip_state));
    });
}

struct SegmentedMeter {
    level: f32,
    clipped: bool,
}

impl SegmentedMeter {
    fn new(level: f32, clipped: bool) -> Self {
        Self { level, clipped }
    }
}

impl egui::Widget for SegmentedMeter {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(METER_WIDTH, METER_HEIGHT), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let level = if self.level.is_finite() {
            self.level.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let fill_right = rect.left() + rect.width() * level;
        let active_color = egui::Color32::from_gray(210);
        let inactive_color = egui::Color32::from_gray(78);
        let clip_active_color = egui::Color32::from_rgb(235, 64, 64);
        let clip_inactive_color = egui::Color32::from_rgb(110, 42, 42);

        for index in 0..METER_SEGMENT_COUNT {
            let left = rect.left() + index as f32 * (METER_SEGMENT_WIDTH + METER_SEGMENT_GAP);
            let segment = egui::Rect::from_min_size(
                egui::pos2(left, rect.top()),
                egui::vec2(METER_SEGMENT_WIDTH, rect.height()),
            );
            let is_clip_segment = index >= METER_SEGMENT_COUNT - METER_CLIP_SEGMENTS;
            let is_active = segment.center().x < fill_right || (self.clipped && is_clip_segment);
            let color = match (is_clip_segment, is_active) {
                (true, true) => clip_active_color,
                (true, false) => clip_inactive_color,
                (false, true) => active_color,
                (false, false) => inactive_color,
            };
            painter.rect_filled(segment, 0.0, color);
        }

        response
    }
}

struct ClipIndicator<'a> {
    state: &'a mut ClipState,
}

impl<'a> ClipIndicator<'a> {
    fn new(state: &'a mut ClipState) -> Self {
        Self { state }
    }
}

impl egui::Widget for ClipIndicator<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(METER_HEIGHT, METER_HEIGHT), egui::Sense::click());
        if response.clicked() {
            self.state.reset();
        }
        let color = if self.state.latched {
            egui::Color32::from_rgb(235, 64, 64)
        } else {
            egui::Color32::from_rgb(110, 42, 42)
        };
        ui.painter_at(rect).rect_filled(rect, 1.0, color);
        response
    }
}

#[derive(Default)]
struct SmoothedLevel {
    samples: VecDeque<f32>,
    sum: f32,
}

impl SmoothedLevel {
    fn update(&mut self, value: f32) -> f32 {
        let value = if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        };
        self.samples.push_back(value);
        self.sum += value;
        if self.samples.len() > METER_SMOOTHING_SAMPLES {
            self.sum -= self.samples.pop_front().unwrap_or_default();
        }
        self.sum / self.samples.len() as f32
    }
}

#[derive(Default)]
struct ClipState {
    latched: bool,
    signal_clipping: bool,
}

impl ClipState {
    fn observe(&mut self, level: f32) {
        let clipping = level.is_finite() && level > 1.0;
        if clipping && !self.signal_clipping {
            self.latched = true;
        }
        self.signal_clipping = clipping;
    }

    fn reset(&mut self) {
        self.latched = false;
    }
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

#[cfg(test)]
mod tests {
    use super::{ClipState, METER_SMOOTHING_SAMPLES, SmoothedLevel};

    #[test]
    fn smooths_recent_level_samples() {
        let mut level = SmoothedLevel::default();
        assert_eq!(level.update(0.0), 0.0);
        assert_eq!(level.update(1.0), 0.5);
        assert_eq!(level.update(1.0), 2.0 / 3.0);
    }

    #[test]
    fn keeps_only_the_recent_sample_window() {
        let mut level = SmoothedLevel::default();
        for _ in 0..METER_SMOOTHING_SAMPLES {
            assert_eq!(level.update(1.0), 1.0);
        }
        assert!(
            (level.update(0.0)
                - (METER_SMOOTHING_SAMPLES - 1) as f32 / METER_SMOOTHING_SAMPLES as f32)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn clip_state_latches_on_a_raw_clip_and_can_be_reset() {
        let mut state = ClipState::default();
        state.observe(1.01);
        assert!(state.latched);
        assert!(state.signal_clipping);

        state.reset();
        state.observe(1.01);
        assert!(!state.latched);

        state.observe(0.5);
        state.observe(1.01);
        assert!(state.latched);
    }
}
