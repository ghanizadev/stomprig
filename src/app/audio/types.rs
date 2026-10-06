#[derive(Clone, Copy, Default)]
pub struct SignalLevels {
    pub input_peak: f32,
    pub output_peak: f32,
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub index: usize,
    pub name: String,
    pub channels: usize,
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AudioBufferSize {
    Samples64,
    Samples128,
    Samples256,
    #[default]
    Samples512,
    Samples1024,
    Samples2048,
}

impl AudioBufferSize {
    pub const ALL: [Self; 6] = [
        Self::Samples64,
        Self::Samples128,
        Self::Samples256,
        Self::Samples512,
        Self::Samples1024,
        Self::Samples2048,
    ];

    pub const fn frames(self) -> u32 {
        match self {
            Self::Samples64 => 64,
            Self::Samples128 => 128,
            Self::Samples256 => 256,
            Self::Samples512 => 512,
            Self::Samples1024 => 1024,
            Self::Samples2048 => 2048,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Samples64 => "64 samples",
            Self::Samples128 => "128 samples",
            Self::Samples256 => "256 samples",
            Self::Samples512 => "512 samples",
            Self::Samples1024 => "1024 samples",
            Self::Samples2048 => "2048 samples",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioBackend, AudioBufferSize, AudioConfig, ChannelRouting};

    #[test]
    fn exposes_predefined_frame_counts() {
        assert_eq!(
            AudioBufferSize::ALL.map(AudioBufferSize::frames),
            [64, 128, 256, 512, 1024, 2048]
        );
    }

    #[test]
    fn audio_config_equality_covers_routing_and_device_settings() {
        let config = AudioConfig {
            backend: AudioBackend::Asio,
            input_index: 1,
            output_index: 2,
            buffer_size: AudioBufferSize::Samples256,
            routing: ChannelRouting {
                input_left: 0,
                input_right: 1,
                output_left: 0,
                output_right: 1,
            },
        };

        assert_eq!(config, config);
        assert_ne!(
            config,
            AudioConfig {
                routing: ChannelRouting {
                    output_right: 2,
                    ..config.routing
                },
                ..config
            }
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AudioBackend {
    #[default]
    Wasapi,
    Asio,
}

impl AudioBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wasapi => "WASAPI",
            Self::Asio => "ASIO",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelRouting {
    pub input_left: usize,
    pub input_right: usize,
    pub output_left: usize,
    pub output_right: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioConfig {
    pub backend: AudioBackend,
    pub input_index: usize,
    pub output_index: usize,
    pub buffer_size: AudioBufferSize,
    pub routing: ChannelRouting,
}

/// The buffering terms that contribute to the estimated input-to-output delay.
///
/// The input and output periods come from the stream configuration selected by
/// CPAL. When a device cannot use the requested fixed buffer, the configured
/// processing block is used as the best available estimate for that device's
/// default period.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLatencyInfo {
    pub sample_rate: f64,
    pub input_buffer_frames: usize,
    pub processing_block_frames: usize,
    pub output_buffer_frames: usize,
}

impl AudioLatencyInfo {
    pub fn estimate_ms(self, plugin_latency_samples: f64) -> Option<f64> {
        if !self.sample_rate.is_finite() || self.sample_rate <= 0.0 {
            return None;
        }

        let input_and_processing_frames =
            self.input_buffer_frames.max(self.processing_block_frames);
        let total_frames =
            input_and_processing_frames.saturating_add(self.output_buffer_frames) as f64;
        let plugin_latency_samples = if plugin_latency_samples.is_finite() {
            plugin_latency_samples.max(0.0)
        } else {
            0.0
        };

        Some((total_frames + plugin_latency_samples) / self.sample_rate * 1000.0)
    }
}

#[cfg(test)]
mod latency_tests {
    use super::AudioLatencyInfo;

    #[test]
    fn estimates_two_matching_audio_periods_and_plugin_latency() {
        let info = AudioLatencyInfo {
            sample_rate: 48_000.0,
            input_buffer_frames: 512,
            processing_block_frames: 512,
            output_buffer_frames: 512,
        };

        assert_eq!(info.estimate_ms(480.0), Some(31.333333333333332));
    }

    #[test]
    fn accounts_for_a_processing_block_larger_than_the_input_period() {
        let info = AudioLatencyInfo {
            sample_rate: 48_000.0,
            input_buffer_frames: 128,
            processing_block_frames: 256,
            output_buffer_frames: 128,
        };

        assert_eq!(info.estimate_ms(0.0), Some(8.0));
    }
}
