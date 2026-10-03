#[derive(Clone, Copy, Default)]
pub struct SignalLevels {
    pub input_peak: f32,
    pub input_rms: f32,
    pub output_peak: f32,
    pub output_rms: f32,
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
