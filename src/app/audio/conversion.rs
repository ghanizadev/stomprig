use cpal::SampleFormat;

pub fn is_supported_sample_format(format: SampleFormat) -> bool {
    matches!(format, SampleFormat::F32 | SampleFormat::I32)
}
pub fn i32_to_f32(sample: i32) -> f32 {
    sample as f32 / 2_147_483_648.0
}
pub fn f32_to_i32(sample: f32) -> i32 {
    if !sample.is_finite() {
        return 0;
    }
    let sample = sample.clamp(-1.0, 1.0);
    if sample <= -1.0 {
        i32::MIN
    } else if sample >= 1.0 {
        i32::MAX
    } else {
        (sample * 2_147_483_648.0).round() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_i32_samples() {
        assert_eq!(i32_to_f32(0), 0.0);
        assert_eq!(i32_to_f32(i32::MIN), -1.0);
        assert!((i32_to_f32(i32::MAX) - 1.0).abs() < 1e-6);
    }
    #[test]
    fn converts_f32_samples() {
        assert_eq!(f32_to_i32(-2.0), i32::MIN);
        assert_eq!(f32_to_i32(-1.0), i32::MIN);
        assert_eq!(f32_to_i32(0.0), 0);
        assert_eq!(f32_to_i32(1.0), i32::MAX);
        assert_eq!(f32_to_i32(2.0), i32::MAX);
        assert_eq!(f32_to_i32(f32::NAN), 0);
    }
}
