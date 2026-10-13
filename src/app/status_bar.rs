use super::audio::{AudioLatencyBreakdown, SignalLevels};
use super::cpu::CpuUsage;
use super::ui::{label, muted};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const METER_WIDTH: f32 = 120.0;
const METER_HEIGHT: f32 = 12.0;
const METER_SEGMENT_WIDTH: f32 = 4.0;
const METER_SEGMENT_GAP: f32 = 1.0;
const METER_SEGMENT_COUNT: usize =
    ((METER_WIDTH + METER_SEGMENT_GAP) / (METER_SEGMENT_WIDTH + METER_SEGMENT_GAP)) as usize;
const METER_CLIP_SEGMENTS: usize = 3;
const METER_SMOOTHING_SAMPLES: usize = 6;
const METER_TOOLTIP_AVERAGE_SAMPLES: usize = 30;
const LATENCY_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct AudioMeterState {
    input_peak: SmoothedLevel,
    output_peak: SmoothedLevel,
    input_average: SmoothedLevel,
    output_average: SmoothedLevel,
    input_clip: ClipState,
    output_clip: ClipState,
}

impl Default for AudioMeterState {
    fn default() -> Self {
        Self {
            input_peak: SmoothedLevel::new(METER_SMOOTHING_SAMPLES),
            output_peak: SmoothedLevel::new(METER_SMOOTHING_SAMPLES),
            input_average: SmoothedLevel::new(METER_TOOLTIP_AVERAGE_SAMPLES),
            output_average: SmoothedLevel::new(METER_TOOLTIP_AVERAGE_SAMPLES),
            input_clip: ClipState::default(),
            output_clip: ClipState::default(),
        }
    }
}

#[derive(Default)]
pub(crate) struct LatencyState {
    breakdown: Option<AudioLatencyBreakdown>,
    last_update: Option<Instant>,
}

impl LatencyState {
    pub(crate) fn refresh(
        &mut self,
        estimate: impl FnOnce() -> Option<AudioLatencyBreakdown>,
    ) -> Option<AudioLatencyBreakdown> {
        let interval_elapsed = self
            .last_update
            .is_none_or(|last_update| last_update.elapsed() >= LATENCY_REFRESH_INTERVAL);
        if interval_elapsed {
            self.breakdown = estimate();
            self.last_update = Some(Instant::now());
        }
        self.breakdown
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    cpu_usage: &mut CpuUsage,
    edit_state: &str,
    loaded_preset: Option<&str>,
    live_preset: Option<&str>,
    levels: Option<SignalLevels>,
    latency_breakdown: Option<AudioLatencyBreakdown>,
    meters: &mut AudioMeterState,
    diagnostic_count: usize,
    dropped_diagnostics: u64,
) -> bool {
    let meter_values = levels.map(|levels| {
        let input_peak = meters.input_peak.update(levels.input_peak);
        let output_peak = meters.output_peak.update(levels.output_peak);
        let input_average = meters.input_average.update(levels.input_peak);
        let output_average = meters.output_average.update(levels.output_peak);
        meters.input_clip.observe(levels.input_peak);
        meters.output_clip.observe(levels.output_peak);
        (
            input_peak,
            input_average,
            meters.input_clip.signal_clipping,
            output_peak,
            output_average,
            meters.output_clip.signal_clipping,
        )
    });

    let diagnostics_clicked = ui
        .horizontal(|ui| {
            muted(ui, edit_state);
            ui.separator();
            if let Some((
                input_peak,
                input_average,
                input_clipping,
                output_peak,
                output_average,
                output_clipping,
            )) = meter_values
            {
                meter(
                    ui,
                    "Input peak",
                    input_peak,
                    input_average,
                    input_clipping,
                    &mut meters.input_clip,
                );
                ui.separator();
                meter(
                    ui,
                    "Output peak",
                    output_peak,
                    output_average,
                    output_clipping,
                    &mut meters.output_clip,
                );
            } else {
                muted(ui, "Input peak --");
                ui.separator();
                muted(ui, "Output peak --");
            }
            ui.separator();
            match cpu_usage.sample() {
                Some(usage) => label(ui, format!("CPU {:.1}%", usage)),
                None => muted(ui, "CPU --"),
            };
            ui.separator();
            let latency_response = match latency_breakdown {
                Some(value) => label(ui, format!("Latency {}", format_latency_ms(value.total_ms))),
                None => muted(ui, "Latency --"),
            };
            latency_response.on_hover_text(format_latency_tooltip(latency_breakdown));
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let diagnostic_label = if dropped_diagnostics > 0 {
                    format!("⚠ ({diagnostic_count}/{dropped_diagnostics})")
                } else if diagnostic_count > 0 {
                    format!("⚠ ({diagnostic_count})")
                } else {
                    "⚠".to_owned()
                };
                let diagnostic_text =
                    egui::RichText::new(diagnostic_label).color(if diagnostic_count > 0 {
                        ui.visuals().warn_fg_color
                    } else {
                        ui.visuals().text_color()
                    });
                let diagnostics_clicked = ui
                    .add(egui::Button::new(diagnostic_text).frame(false))
                    .on_hover_text("View startup warnings, audio errors, and plug-in incidents")
                    .clicked();
                if let Some(preset) = loaded_preset {
                    let preset_name = preset.rsplit('/').next().unwrap_or(preset).trim();
                    label(ui, format!("Preset: {preset_name}")).on_hover_text(preset);
                    ui.separator();
                }
                if let Some(live_preset) = live_preset {
                    let live_name = live_preset.rsplit('/').next().unwrap_or(live_preset).trim();
                    label(ui, format!("Live: {live_name}")).on_hover_text(live_preset);
                    ui.separator();
                }
                diagnostics_clicked
            })
            .inner
        })
        .inner;
    ui.ctx()
        .request_repaint_after(Duration::from_millis(if levels.is_some() {
            33
        } else {
            500
        }));
    diagnostics_clicked
}

fn format_latency_ms(value: f64) -> String {
    let Some(rounded) = round_to_significant_digits(value, 2) else {
        return "--".to_owned();
    };
    if rounded == 0.0 {
        return "0 ms".to_owned();
    }

    let exponent = rounded.abs().log10().floor() as i32;
    let decimal_places = (1 - exponent).max(0) as usize;
    format!("{rounded:.decimal_places$} ms")
}

fn format_latency_tooltip(breakdown: Option<AudioLatencyBreakdown>) -> String {
    let Some(breakdown) = breakdown else {
        return "Estimated round-trip latency: input period + processing block + output period + queued audio + plugin latency".to_owned();
    };

    format!(
        "Estimated round-trip latency: input period ({}) + processing block ({}) + output period ({}) + queued audio ({}: input {}, output {}) + plugin latency ({}) = {}",
        format_latency_ms(breakdown.input_period_ms),
        format_latency_ms(breakdown.processing_block_ms),
        format_latency_ms(breakdown.output_period_ms),
        format_latency_ms(breakdown.queued_audio_ms),
        format_latency_ms(breakdown.input_queue_ms),
        format_latency_ms(breakdown.output_queue_ms),
        format_latency_ms(breakdown.plugin_latency_ms),
        format_latency_ms(breakdown.total_ms),
    )
}

fn round_to_significant_digits(value: f64, digits: i32) -> Option<f64> {
    if !value.is_finite() || digits < 1 {
        return None;
    }
    if value == 0.0 {
        return Some(0.0);
    }

    let exponent = value.abs().log10().floor() as i32;
    let scale = 10_f64.powi(digits - 1 - exponent);
    Some((value * scale).round() / scale)
}

fn meter(
    ui: &mut egui::Ui,
    label: &str,
    level: f32,
    average_level: f32,
    clipping: bool,
    clip_state: &mut ClipState,
) {
    let response = ui
        .horizontal(|ui| {
            super::ui::label(ui, label);
            ui.add(SegmentedMeter::new(level, clipping));
            ui.add_space(2.0);
            ui.add(ClipIndicator::new(clip_state));
        })
        .response;
    response.on_hover_text(format_decibels(average_level));
}

fn format_decibels(level: f32) -> String {
    if level > 0.0 {
        format!("{:.1} dB", 20.0 * level.log10())
    } else {
        "-\u{221e} dB".to_owned()
    }
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

struct SmoothedLevel {
    samples: VecDeque<f32>,
    sum: f32,
    sample_count: usize,
}

impl SmoothedLevel {
    fn new(sample_count: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(sample_count),
            sum: 0.0,
            sample_count,
        }
    }

    fn update(&mut self, value: f32) -> f32 {
        let value = if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        };
        self.samples.push_back(value);
        self.sum += value;
        if self.samples.len() > self.sample_count {
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
        let clipping = level.is_finite() && level >= 1.0;
        if clipping && !self.signal_clipping {
            self.latched = true;
        }
        self.signal_clipping = clipping;
    }

    fn reset(&mut self) {
        self.latched = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioLatencyBreakdown, ClipState, LATENCY_REFRESH_INTERVAL, LatencyState, format_decibels,
        format_latency_ms, round_to_significant_digits,
    };
    use std::time::Instant;

    #[test]
    fn formats_peak_levels_as_decibels() {
        assert_eq!(format_decibels(1.0), "0.0 dB");
        assert_eq!(format_decibels(0.5), "-6.0 dB");
        assert_eq!(format_decibels(0.0), "-∞ dB");
    }

    #[test]
    fn clip_state_latches_on_a_raw_clip_and_can_be_reset() {
        let mut state = ClipState::default();

        state.observe(1.0);
        assert!(state.latched);

        state.reset();
        assert!(!state.latched);

        state.observe(0.5);
        state.observe(1.01);
        assert!(state.latched);
    }

    #[test]
    fn clip_state_does_not_latch_below_zero_dbfs() {
        let mut state = ClipState::default();

        state.observe(0.999_999);

        assert!(!state.latched);
    }

    #[test]
    fn rounds_latency_to_two_significant_digits() {
        assert_eq!(format_latency_ms(21.333), "21 ms");
        assert_eq!(format_latency_ms(1.234), "1.2 ms");
        assert_eq!(format_latency_ms(0.1234), "0.12 ms");
        assert_eq!(format_latency_ms(0.0), "0 ms");
    }

    #[test]
    fn rejects_non_finite_latency_values() {
        assert_eq!(round_to_significant_digits(f64::NAN, 2), None);
        assert_eq!(round_to_significant_digits(f64::INFINITY, 2), None);
    }

    #[test]
    fn latency_state_samples_at_most_once_per_refresh_interval() {
        let mut state = LatencyState::default();
        let first = AudioLatencyBreakdown {
            total_ms: 10.0,
            input_period_ms: 0.0,
            processing_block_ms: 0.0,
            output_period_ms: 0.0,
            queued_audio_ms: 0.0,
            input_queue_ms: 0.0,
            output_queue_ms: 0.0,
            plugin_latency_ms: 0.0,
        };
        let second = AudioLatencyBreakdown {
            total_ms: 20.0,
            ..first
        };

        assert_eq!(state.refresh(|| Some(first)), Some(first));
        assert_eq!(
            state.refresh(|| panic!("sampled before the interval elapsed")),
            Some(first)
        );

        state.last_update = Some(Instant::now() - LATENCY_REFRESH_INTERVAL);
        assert_eq!(state.refresh(|| Some(second)), Some(second));
    }
}
