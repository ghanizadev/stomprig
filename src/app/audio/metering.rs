use super::types::SignalLevels;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct LevelAtomics {
    pub input_peak: AtomicU32,
    pub output_peak: AtomicU32,
}
impl LevelAtomics {
    pub fn new() -> Self {
        Self {
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
        }
    }
    pub fn read(&self) -> SignalLevels {
        SignalLevels {
            input_peak: read(&self.input_peak),
            output_peak: read(&self.output_peak),
        }
    }
}
fn read(value: &AtomicU32) -> f32 {
    f32::from_bits(value.load(Ordering::Relaxed))
}
pub fn update_levels(samples: &[f32], peak: &AtomicU32) {
    let max = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0, f32::max);
    peak.store(max.to_bits(), Ordering::Relaxed);
}
pub fn update_channel_levels(channels: &[Vec<f32>], frames: usize, peak: &AtomicU32) {
    let mut max: f32 = 0.0;
    for channel in channels {
        for &sample in channel.iter().take(frames) {
            max = max.max(sample.abs());
        }
    }
    peak.store(max.to_bits(), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calculates_levels() {
        let peak = AtomicU32::new(0);
        update_levels(&[-1.0, 0.5, -0.5, 0.0], &peak);
        assert_eq!(read(&peak), 1.0);
    }
    #[test]
    fn empty_is_silent() {
        let peak = AtomicU32::new(1.0f32.to_bits());
        update_levels(&[], &peak);
        assert_eq!(read(&peak), 0.0);
    }
}
