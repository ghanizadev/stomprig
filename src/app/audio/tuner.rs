use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

const HISTORY_CAPACITY: usize = 4096;
const MIN_FREQUENCY_HZ: f32 = 30.0;
const MAX_FREQUENCY_HZ: f32 = 1_000.0;
const ANALYSIS_INTERVAL_BLOCKS: usize = 4;
pub(crate) const DEFAULT_INPUT_SENSITIVITY_DB: f32 = -50.0;
pub(crate) const MIN_INPUT_SENSITIVITY_DB: f32 = -80.0;
pub(crate) const MAX_INPUT_SENSITIVITY_DB: f32 = -20.0;
pub(crate) const DEFAULT_A4_PITCH_HZ: f32 = 440.0;
pub(crate) const MIN_A4_PITCH_HZ: f32 = 400.0;
pub(crate) const MAX_A4_PITCH_HZ: f32 = 480.0;
pub(crate) const DEFAULT_TRANSPOSITION_SEMITONES: i32 = 0;
pub(crate) const MIN_TRANSPOSITION_SEMITONES: i32 = -12;
pub(crate) const MAX_TRANSPOSITION_SEMITONES: i32 = 12;
// Keeping every second sample gives the detector a 24 kHz analysis rate while
// keeping the autocorrelation work bounded for the audio worker thread.
const ANALYSIS_DOWNSAMPLE: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningPreset {
    JamesTaylor,
    #[default]
    StandardElectricGuitar,
    Flat,
    BassAttackCompensation,
    BuzzFeiten,
}

impl TuningPreset {
    pub const ALL: [Self; 5] = [
        Self::StandardElectricGuitar,
        Self::Flat,
        Self::JamesTaylor,
        Self::BassAttackCompensation,
        Self::BuzzFeiten,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::JamesTaylor => "James Taylor Acoustic Guitar",
            Self::StandardElectricGuitar => "Standard Electric Guitar",
            Self::Flat => "Flat (No Sweetening)",
            Self::BassAttackCompensation => "Bass Attack Compensation",
            Self::BuzzFeiten => "Buzz Feiten",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "James Taylor Acoustic Guitar" => Some(Self::JamesTaylor),
            "Standard Electric Guitar" => Some(Self::StandardElectricGuitar),
            "Flat (No Sweetening)" => Some(Self::Flat),
            "Bass Attack Compensation" => Some(Self::BassAttackCompensation),
            "Buzz Feiten" => Some(Self::BuzzFeiten),
            _ => None,
        }
    }

    fn as_u32(self) -> u32 {
        match self {
            Self::JamesTaylor => 0,
            Self::StandardElectricGuitar => 1,
            Self::Flat => 2,
            Self::BassAttackCompensation => 3,
            Self::BuzzFeiten => 4,
        }
    }

    fn from_u32(value: u32) -> Self {
        match value {
            0 => Self::JamesTaylor,
            3 => Self::BassAttackCompensation,
            4 => Self::BuzzFeiten,
            _ => Self::StandardElectricGuitar,
        }
    }

    /// Returns the cent offset for a standard-tuned instrument's open string.
    /// The tuner subtracts this value from the raw equal-temperament deviation.
    fn offset_for_midi_note(self, midi_note: i32) -> f32 {
        match self {
            Self::JamesTaylor => match midi_note {
                40 => -12.0, // Low E
                45 => -10.0, // A
                50 => -8.0,  // D
                55 => -4.0,  // G
                59 => -6.0,  // B
                64 => -3.0,  // High E
                _ => 0.0,
            },
            Self::StandardElectricGuitar => match midi_note {
                55 => -3.0, // G
                59 => -2.0, // B
                _ => 0.0,
            },
            Self::Flat => 0.0,
            Self::BassAttackCompensation => match midi_note {
                28 | 35 => 2.0, // Low E / low B
                33 => 1.0,      // A
                _ => 0.0,       // D / G
            },
            Self::BuzzFeiten => match midi_note {
                45 | 50 => 1.0, // A / D
                55 => 2.0,      // G
                _ => 0.0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TunerReadout {
    pub frequency_hz: f32,
    pub cents: f32,
    pub confidence: f32,
}

pub struct TunerState {
    bypassed: AtomicBool,
    frequency_hz: AtomicU32,
    cents: AtomicU32,
    confidence: AtomicU32,
    input_sensitivity_db: AtomicU32,
    a4_pitch_hz: AtomicU32,
    transposition_semitones: AtomicI32,
    tuning_preset: AtomicU32,
}

impl TunerState {
    pub fn new() -> Self {
        Self {
            bypassed: AtomicBool::new(false),
            frequency_hz: AtomicU32::new(0.0f32.to_bits()),
            cents: AtomicU32::new(0.0f32.to_bits()),
            confidence: AtomicU32::new(0.0f32.to_bits()),
            input_sensitivity_db: AtomicU32::new(DEFAULT_INPUT_SENSITIVITY_DB.to_bits()),
            a4_pitch_hz: AtomicU32::new(DEFAULT_A4_PITCH_HZ.to_bits()),
            transposition_semitones: AtomicI32::new(DEFAULT_TRANSPOSITION_SEMITONES),
            tuning_preset: AtomicU32::new(TuningPreset::default().as_u32()),
        }
    }

    pub fn is_bypassed(&self) -> bool {
        self.bypassed.load(Ordering::Relaxed)
    }

    pub fn set_bypassed(&self, bypassed: bool) {
        self.bypassed.store(bypassed, Ordering::Relaxed);
        if bypassed {
            self.clear_readout();
        }
    }

    pub fn set_input_sensitivity_db(&self, sensitivity_db: f32) {
        let sensitivity_db = if sensitivity_db.is_finite() {
            sensitivity_db.clamp(MIN_INPUT_SENSITIVITY_DB, MAX_INPUT_SENSITIVITY_DB)
        } else {
            DEFAULT_INPUT_SENSITIVITY_DB
        };
        self.input_sensitivity_db
            .store(sensitivity_db.to_bits(), Ordering::Relaxed);
    }

    pub fn input_sensitivity_db(&self) -> f32 {
        let sensitivity_db = f32::from_bits(self.input_sensitivity_db.load(Ordering::Relaxed));
        if sensitivity_db.is_finite() {
            sensitivity_db.clamp(MIN_INPUT_SENSITIVITY_DB, MAX_INPUT_SENSITIVITY_DB)
        } else {
            DEFAULT_INPUT_SENSITIVITY_DB
        }
    }

    pub fn set_a4_pitch_hz(&self, pitch_hz: f32) {
        let pitch_hz = if pitch_hz.is_finite() {
            pitch_hz.clamp(MIN_A4_PITCH_HZ, MAX_A4_PITCH_HZ)
        } else {
            DEFAULT_A4_PITCH_HZ
        };
        self.a4_pitch_hz
            .store(pitch_hz.to_bits(), Ordering::Relaxed);
    }

    pub fn a4_pitch_hz(&self) -> f32 {
        let pitch_hz = f32::from_bits(self.a4_pitch_hz.load(Ordering::Relaxed));
        if pitch_hz.is_finite() {
            pitch_hz.clamp(MIN_A4_PITCH_HZ, MAX_A4_PITCH_HZ)
        } else {
            DEFAULT_A4_PITCH_HZ
        }
    }

    pub fn set_transposition_semitones(&self, semitones: i32) {
        self.transposition_semitones.store(
            semitones.clamp(MIN_TRANSPOSITION_SEMITONES, MAX_TRANSPOSITION_SEMITONES),
            Ordering::Relaxed,
        );
    }

    pub fn transposition_semitones(&self) -> i32 {
        self.transposition_semitones
            .load(Ordering::Relaxed)
            .clamp(MIN_TRANSPOSITION_SEMITONES, MAX_TRANSPOSITION_SEMITONES)
    }

    pub fn set_tuning_preset(&self, preset: TuningPreset) {
        self.tuning_preset.store(preset.as_u32(), Ordering::Relaxed);
    }

    pub fn tuning_preset(&self) -> TuningPreset {
        TuningPreset::from_u32(self.tuning_preset.load(Ordering::Relaxed))
    }

    pub fn read(&self) -> TunerReadout {
        TunerReadout {
            frequency_hz: f32::from_bits(self.frequency_hz.load(Ordering::Relaxed)),
            cents: f32::from_bits(self.cents.load(Ordering::Relaxed)),
            confidence: f32::from_bits(self.confidence.load(Ordering::Relaxed)),
        }
    }

    fn clear_readout(&self) {
        self.frequency_hz.store(0.0f32.to_bits(), Ordering::Relaxed);
        self.cents.store(0.0f32.to_bits(), Ordering::Relaxed);
        self.confidence.store(0.0f32.to_bits(), Ordering::Relaxed);
    }

    fn update(&self, readout: TunerReadout) {
        self.frequency_hz
            .store(readout.frequency_hz.to_bits(), Ordering::Relaxed);
        self.cents.store(readout.cents.to_bits(), Ordering::Relaxed);
        self.confidence
            .store(readout.confidence.to_bits(), Ordering::Relaxed);
    }
}

impl Default for TunerState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TunerProcessor {
    history: [f32; HISTORY_CAPACITY],
    write_index: usize,
    filled: usize,
    blocks_since_analysis: usize,
    sample_rate: f32,
}

impl TunerProcessor {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            history: [0.0; HISTORY_CAPACITY],
            write_index: 0,
            filled: 0,
            blocks_since_analysis: ANALYSIS_INTERVAL_BLOCKS,
            sample_rate: sample_rate as f32,
        }
    }

    pub fn process(&mut self, inputs: &[Vec<f32>], frames: usize, state: &TunerState) {
        if state.is_bypassed() {
            self.write_index = 0;
            self.filled = 0;
            self.blocks_since_analysis = ANALYSIS_INTERVAL_BLOCKS;
            state.clear_readout();
            return;
        }

        let left = inputs.first();
        let right = inputs.get(1);
        for frame in 0..frames {
            let left_sample = left
                .and_then(|channel| channel.get(frame))
                .copied()
                .unwrap_or(0.0);
            let right_sample = right
                .and_then(|channel| channel.get(frame))
                .copied()
                .unwrap_or(left_sample);
            self.history[self.write_index] = (left_sample + right_sample) * 0.5;
            self.write_index = (self.write_index + 1) % HISTORY_CAPACITY;
            self.filled = (self.filled + 1).min(HISTORY_CAPACITY);
        }

        self.blocks_since_analysis += 1;
        if self.blocks_since_analysis < ANALYSIS_INTERVAL_BLOCKS {
            return;
        }
        self.blocks_since_analysis = 0;

        let Some(readout) = self.analyse(
            input_rms_threshold(state.input_sensitivity_db()),
            state.a4_pitch_hz(),
            state.transposition_semitones(),
            state.tuning_preset(),
        ) else {
            state.clear_readout();
            return;
        };
        state.update(readout);
    }

    fn analyse(
        &self,
        input_rms_threshold: f32,
        a4_pitch_hz: f32,
        transposition_semitones: i32,
        tuning_preset: TuningPreset,
    ) -> Option<TunerReadout> {
        if self.filled < 512 * ANALYSIS_DOWNSAMPLE
            || !self.sample_rate.is_finite()
            || self.sample_rate <= 0.0
        {
            return None;
        }

        let window = self.filled / ANALYSIS_DOWNSAMPLE;
        let sample_rate = self.sample_rate / ANALYSIS_DOWNSAMPLE as f32;
        let mut mean = 0.0;
        for index in 0..window {
            mean += self.sample_at(index * ANALYSIS_DOWNSAMPLE);
        }
        mean /= window as f32;

        let mut energy = 0.0;
        for index in 0..window {
            let sample = self.sample_at(index * ANALYSIS_DOWNSAMPLE) - mean;
            energy += sample * sample;
        }
        let rms = (energy / window as f32).sqrt();
        if rms < input_rms_threshold {
            return None;
        }

        let min_lag = (sample_rate / MAX_FREQUENCY_HZ).round().max(2.0) as usize;
        let max_lag = (sample_rate / MIN_FREQUENCY_HZ).round() as usize;
        let max_lag = max_lag.min(window.saturating_sub(128));
        if min_lag >= max_lag {
            return None;
        }

        let mut best_lag = 0;
        let mut best_difference = f32::MAX;
        let mut first_good_lag = None;
        let mut first_good_difference = f32::MAX;
        let normalizer = energy / window as f32;
        let mut previous_previous = f32::MAX;
        let mut previous = f32::MAX;
        for lag in min_lag..=max_lag {
            let normalized = self.difference_at(window, mean, lag);
            if first_good_lag.is_none()
                && lag >= min_lag + 2
                && previous <= previous_previous
                && previous <= normalized
                && previous < normalizer * 0.5
            {
                first_good_lag = Some(lag - 1);
                first_good_difference = previous;
            }
            if normalized < best_difference {
                best_difference = normalized;
                best_lag = lag;
            }
            previous_previous = previous;
            previous = normalized;
        }
        if let Some(lag) = first_good_lag {
            best_lag = lag;
            best_difference = first_good_difference;
        }
        if best_lag == 0 {
            return None;
        }

        // The integer lag gets us close; a parabolic interpolation around the
        // difference minimum recovers the fractional period between samples.
        // This removes the large pitch steps that an integer-only detector has
        // at higher notes.
        let mut precise_lag = best_lag as f32;
        if best_lag > min_lag && best_lag < max_lag {
            let previous = self.difference_at(window, mean, best_lag - 1);
            let next = self.difference_at(window, mean, best_lag + 1);
            let denominator = previous - 2.0 * best_difference + next;
            if denominator.is_finite() && denominator.abs() > f32::EPSILON {
                let offset = (0.5 * (previous - next) / denominator).clamp(-0.5, 0.5);
                precise_lag += offset;
            }
        }

        let confidence = (1.0 - best_difference / (normalizer * 2.0)).clamp(0.0, 1.0);
        if confidence < 0.25 {
            return None;
        }
        let frequency_hz = sample_rate / precise_lag;
        if !(MIN_FREQUENCY_HZ..=MAX_FREQUENCY_HZ).contains(&frequency_hz) {
            return None;
        }

        Some(TunerReadout {
            frequency_hz,
            cents: tuning_cents_for_preset(
                frequency_hz,
                a4_pitch_hz,
                transposition_semitones,
                tuning_preset,
            ),
            confidence,
        })
    }

    fn difference_at(&self, window: usize, mean: f32, lag: usize) -> f32 {
        let overlap = window - lag;
        let mut difference = 0.0;
        for index in 0..overlap {
            let first = self.sample_at(index * ANALYSIS_DOWNSAMPLE) - mean;
            let second = self.sample_at((index + lag) * ANALYSIS_DOWNSAMPLE) - mean;
            let delta = first - second;
            difference += delta * delta;
        }
        difference / overlap as f32
    }

    fn sample_at(&self, offset: usize) -> f32 {
        let first = (self.write_index + HISTORY_CAPACITY - self.filled) % HISTORY_CAPACITY;
        self.history[(first + offset) % HISTORY_CAPACITY]
    }
}

fn input_rms_threshold(sensitivity_db: f32) -> f32 {
    10.0f32.powf(sensitivity_db / 20.0)
}

#[allow(dead_code)]
fn tuning_cents(frequency_hz: f32, a4_pitch_hz: f32, transposition_semitones: i32) -> f32 {
    let note_number =
        69.0 + 12.0 * (frequency_hz / a4_pitch_hz).log2() + transposition_semitones as f32;
    (note_number - note_number.round()) * 100.0
}

fn tuning_cents_for_preset(
    frequency_hz: f32,
    a4_pitch_hz: f32,
    transposition_semitones: i32,
    tuning_preset: TuningPreset,
) -> f32 {
    let note_number =
        69.0 + 12.0 * (frequency_hz / a4_pitch_hz).log2() + transposition_semitones as f32;
    let raw_cents = (note_number - note_number.round()) * 100.0;
    raw_cents - tuning_preset.offset_for_midi_note(note_number.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::{TunerProcessor, TunerState, TuningPreset, tuning_cents, tuning_cents_for_preset};

    #[test]
    fn concert_a_is_centered() {
        assert_eq!(tuning_cents(440.0, 440.0, 0), 0.0);
    }

    #[test]
    fn cents_are_measured_from_the_nearest_note() {
        let cents = tuning_cents(445.0, 440.0, 0);
        assert!(cents > 15.0 && cents < 20.0);
    }

    #[test]
    fn custom_a4_is_centered() {
        assert_eq!(tuning_cents(432.0, 432.0, 0), 0.0);
    }

    #[test]
    fn transposition_shifts_the_displayed_pitch() {
        assert_eq!(tuning_cents(440.0, 440.0, 1), 0.0);
    }

    #[test]
    fn sweetened_preset_moves_the_zero_point() {
        let frequency_hz = 440.0 * 2.0f32.powf((55.0 - 69.0 - 4.0 / 100.0) / 12.0);
        let cents = tuning_cents_for_preset(frequency_hz, 440.0, 0, TuningPreset::JamesTaylor);
        assert!(cents.abs() < 0.01, "cents={cents}");
    }

    #[test]
    fn default_preset_is_standard_electric_guitar() {
        assert_eq!(
            TuningPreset::default(),
            TuningPreset::StandardElectricGuitar
        );
    }

    #[test]
    fn flat_preset_keeps_equal_temperament_offsets() {
        let frequency_hz = 440.0 * 2.0f32.powf((55.0 - 69.0 - 4.0 / 100.0) / 12.0);
        let cents = tuning_cents_for_preset(frequency_hz, 440.0, 0, TuningPreset::Flat);
        assert!((cents + 4.0).abs() < 0.01, "cents={cents}");
    }

    #[test]
    fn detects_a_clean_reference_tone() {
        let readout = detect_tone(440.0);
        assert!(
            (readout.frequency_hz - 440.0).abs() < 0.5,
            "readout={readout:?}"
        );
        assert!(readout.cents.abs() < 2.0, "readout={readout:?}");
        assert!(readout.confidence > 0.25);
    }

    #[test]
    fn detects_a_low_instrument_tone() {
        let readout = detect_tone(82.41);
        assert!(
            (readout.frequency_hz - 82.41).abs() < 0.2,
            "readout={readout:?}"
        );
        assert!(readout.cents.abs() < 2.0, "readout={readout:?}");
        assert!(readout.confidence > 0.25);
    }

    #[test]
    fn ignores_quiet_input_noise() {
        let readout = detect_tone_at_amplitude(440.0, 0.001);
        assert_eq!(readout.frequency_hz, 0.0);
        assert_eq!(readout.confidence, 0.0);
    }

    fn detect_tone(frequency_hz: f32) -> super::TunerReadout {
        detect_tone_at_amplitude(frequency_hz, 0.5)
    }

    fn detect_tone_at_amplitude(frequency_hz: f32, amplitude: f32) -> super::TunerReadout {
        let mut processor = TunerProcessor::new(48_000.0);
        let state = TunerState::new();
        let mut phase: f32 = 0.0;
        let phase_step = std::f32::consts::TAU * frequency_hz / 48_000.0;
        for _ in 0..48 {
            let mut left = Vec::with_capacity(128);
            let mut right = Vec::with_capacity(128);
            for _ in 0..128 {
                let sample = phase.sin() * amplitude;
                left.push(sample);
                right.push(sample);
                phase += phase_step;
            }
            processor.process(&[left, right], 128, &state);
        }
        state.read()
    }
}
