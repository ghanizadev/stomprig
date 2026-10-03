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
    use super::AudioBufferSize;

    #[test]
    fn exposes_predefined_frame_counts() {
        assert_eq!(
            AudioBufferSize::ALL.map(AudioBufferSize::frames),
            [64, 128, 256, 512, 1024, 2048]
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

#[derive(Clone, Copy, Debug)]
pub struct ChannelRouting {
    pub input_left: usize,
    pub input_right: usize,
    pub output_left: usize,
    pub output_right: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct AudioConfig {
    pub backend: AudioBackend,
    pub input_index: usize,
    pub output_index: usize,
    pub buffer_size: AudioBufferSize,
    pub routing: ChannelRouting,
}
