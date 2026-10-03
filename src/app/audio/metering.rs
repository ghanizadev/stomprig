use super::types::SignalLevels;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct LevelAtomics {
    pub input_peak: AtomicU32,
    pub input_rms: AtomicU32,
    pub output_peak: AtomicU32,
    pub output_rms: AtomicU32,
}
impl LevelAtomics {
    pub fn new() -> Self {
        Self {
            input_peak: AtomicU32::new(0),
            input_rms: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            output_rms: AtomicU32::new(0),
        }
    }
    pub fn read(&self) -> SignalLevels {
        SignalLevels {
            input_peak: read(&self.input_peak),
            input_rms: read(&self.input_rms),
            output_peak: read(&self.output_peak),
            output_rms: read(&self.output_rms),
        }
    }
}
fn read(value: &AtomicU32) -> f32 {
    f32::from_bits(value.load(Ordering::Relaxed))
}
pub fn update_levels(samples: &[f32], peak: &AtomicU32, rms: &AtomicU32) {
    let max = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0, f32::max);
    let mean_square = if samples.is_empty() {
        0.0
    } else {
        samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32
    };
    peak.store(max.to_bits(), Ordering::Relaxed);
    rms.store(mean_square.sqrt().to_bits(), Ordering::Relaxed);
}
pub fn update_channel_levels(
    channels: &[Vec<f32>],
    frames: usize,
    peak: &AtomicU32,
    rms: &AtomicU32,
) {
    let mut max: f32 = 0.0;
    let mut sum = 0.0;
    let mut count = 0;
    for channel in channels {
        for &sample in channel.iter().take(frames) {
            max = max.max(sample.abs());
            sum += sample * sample;
            count += 1;
        }
    }
    peak.store(max.to_bits(), Ordering::Relaxed);
    rms.store(
        if count == 0 {
            0.0
        } else {
            (sum / count as f32).sqrt()
        }
        .to_bits(),
        Ordering::Relaxed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calculates_levels() {
        let peak = AtomicU32::new(0);
        let rms = AtomicU32::new(0);
        update_levels(&[-1.0, 0.5, -0.5, 0.0], &peak, &rms);
        assert_eq!(read(&peak), 1.0);
        assert!((read(&rms) - 0.6123724).abs() < 0.0001);
    }
    #[test]
    fn empty_is_silent() {
        let peak = AtomicU32::new(1.0f32.to_bits());
        let rms = AtomicU32::new(1.0f32.to_bits());
        update_levels(&[], &peak, &rms);
        assert_eq!(read(&peak), 0.0);
        assert_eq!(read(&rms), 0.0);
    }
}
