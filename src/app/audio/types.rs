#[derive(Clone, Copy, Default)]
pub struct SignalLevels {
    pub input_peak: f32,
    pub output_peak: f32,
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub id: String,
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
            input_device_id: "asio:input-driver".to_owned(),
            output_device_id: "asio:output-driver".to_owned(),
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
                ..config.clone()
            }
        );
        assert_ne!(
            config,
            AudioConfig {
                input_device_id: "asio:other-input-driver".to_owned(),
                ..config.clone()
            }
        );
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AudioBackend {
    Wasapi,
    Asio,
    Alsa,
    Jack,
    PipeWire,
    CoreAudio,
}

impl AudioBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wasapi => "WASAPI",
            Self::Asio => "ASIO",
            Self::Alsa => "ALSA",
            Self::Jack => "JACK / PipeWire",
            Self::PipeWire => "PipeWire",
            Self::CoreAudio => "CoreAudio",
        }
    }

    pub(crate) fn from_host_id(host_id: cpal::HostId) -> Option<Self> {
        match host_id.to_string().to_ascii_lowercase().as_str() {
            #[cfg(windows)]
            "wasapi" => Some(Self::Wasapi),
            #[cfg(windows)]
            "asio" => Some(Self::Asio),
            #[cfg(target_os = "linux")]
            "alsa" => Some(Self::Alsa),
            #[cfg(target_os = "linux")]
            "jack" => Some(Self::Jack),
            #[cfg(target_os = "linux")]
            "pipewire" => Some(Self::PipeWire),
            #[cfg(target_os = "macos")]
            "coreaudio" => Some(Self::CoreAudio),
            _ => None,
        }
    }

    pub(crate) fn host_id(self) -> Result<cpal::HostId, String> {
        use std::str::FromStr;

        let host_name = match self {
            Self::Wasapi => "wasapi",
            Self::Asio => "asio",
            Self::Alsa => "alsa",
            Self::Jack => "jack",
            Self::PipeWire => "pipewire",
            Self::CoreAudio => "coreaudio",
        };
        cpal::HostId::from_str(host_name)
            .map_err(|_| format!("Audio host {host_name} is unavailable on this platform"))
    }

    pub(crate) fn fallback() -> Self {
        #[cfg(windows)]
        {
            Self::Wasapi
        }
        #[cfg(target_os = "linux")]
        {
            Self::Alsa
        }
        #[cfg(target_os = "macos")]
        {
            Self::CoreAudio
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            Self::Wasapi
        }
    }

    pub(crate) const fn is_asio(self) -> bool {
        matches!(self, Self::Asio)
    }
}

impl Default for AudioBackend {
    fn default() -> Self {
        Self::fallback()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelRouting {
    pub input_left: usize,
    pub input_right: usize,
    pub output_left: usize,
    pub output_right: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioConfig {
    pub backend: AudioBackend,
    pub input_device_id: String,
    pub output_device_id: String,
    pub buffer_size: AudioBufferSize,
    pub routing: ChannelRouting,
}

/// The buffering terms that contribute to the estimated input-to-output delay.
///
/// The input and output periods come from the stream configuration selected by
/// CPAL. When a device cannot use the requested fixed buffer, the configured
/// processing block is used as the best available estimate for that device's
/// default period. Queue frames are measured from the live transport queues.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLatencyInfo {
    pub sample_rate: f64,
    pub input_buffer_frames: usize,
    pub processing_block_frames: usize,
    pub output_buffer_frames: usize,
    pub input_queue_frames: usize,
    pub output_queue_frames: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLatencyBreakdown {
    pub total_ms: f64,
    pub input_period_ms: f64,
    pub processing_block_ms: f64,
    pub output_period_ms: f64,
    pub queued_audio_ms: f64,
    pub input_queue_ms: f64,
    pub output_queue_ms: f64,
    pub plugin_latency_ms: f64,
}

impl AudioLatencyInfo {
    pub fn estimate_breakdown_ms(
        self,
        plugin_latency_samples: f64,
    ) -> Option<AudioLatencyBreakdown> {
        if !self.sample_rate.is_finite() || self.sample_rate <= 0.0 {
            return None;
        }

        let input_queue_frames = self.input_queue_frames;
        let output_queue_frames = self.output_queue_frames;
        let total_frames = self
            .input_buffer_frames
            .saturating_add(self.processing_block_frames)
            .saturating_add(self.output_buffer_frames)
            .saturating_add(input_queue_frames)
            .saturating_add(output_queue_frames) as f64;
        let plugin_latency_samples = if plugin_latency_samples.is_finite() {
            plugin_latency_samples.max(0.0)
        } else {
            0.0
        };

        let frames_to_ms = |frames: usize| frames as f64 / self.sample_rate * 1000.0;
        Some(AudioLatencyBreakdown {
            total_ms: (total_frames + plugin_latency_samples) / self.sample_rate * 1000.0,
            input_period_ms: frames_to_ms(self.input_buffer_frames),
            processing_block_ms: frames_to_ms(self.processing_block_frames),
            output_period_ms: frames_to_ms(self.output_buffer_frames),
            queued_audio_ms: frames_to_ms(input_queue_frames.saturating_add(output_queue_frames)),
            input_queue_ms: frames_to_ms(input_queue_frames),
            output_queue_ms: frames_to_ms(output_queue_frames),
            plugin_latency_ms: plugin_latency_samples / self.sample_rate * 1000.0,
        })
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
            input_queue_frames: 0,
            output_queue_frames: 0,
        };

        assert_eq!(
            info.estimate_breakdown_ms(480.0)
                .map(|breakdown| breakdown.total_ms),
            Some(42.0)
        );
    }

    #[test]
    fn accounts_for_a_processing_block_larger_than_the_input_period() {
        let info = AudioLatencyInfo {
            sample_rate: 48_000.0,
            input_buffer_frames: 128,
            processing_block_frames: 256,
            output_buffer_frames: 128,
            input_queue_frames: 0,
            output_queue_frames: 0,
        };

        assert_eq!(
            info.estimate_breakdown_ms(0.0)
                .map(|breakdown| breakdown.total_ms),
            Some(10.666666666666666)
        );
    }

    #[test]
    fn includes_queued_audio_in_the_estimate() {
        let info = AudioLatencyInfo {
            sample_rate: 48_000.0,
            input_buffer_frames: 128,
            processing_block_frames: 256,
            output_buffer_frames: 128,
            input_queue_frames: 512,
            output_queue_frames: 256,
        };

        assert_eq!(
            info.estimate_breakdown_ms(0.0)
                .map(|breakdown| breakdown.total_ms),
            Some(26.666666666666668)
        );
    }

    #[test]
    fn breaks_round_trip_latency_into_its_components() {
        let info = AudioLatencyInfo {
            sample_rate: 48_000.0,
            input_buffer_frames: 48,
            processing_block_frames: 96,
            output_buffer_frames: 144,
            input_queue_frames: 240,
            output_queue_frames: 336,
        };

        assert_eq!(
            info.estimate_breakdown_ms(480.0),
            Some(super::AudioLatencyBreakdown {
                total_ms: 28.0,
                input_period_ms: 1.0,
                processing_block_ms: 2.0,
                output_period_ms: 3.0,
                queued_audio_ms: 12.0,
                input_queue_ms: 5.0,
                output_queue_ms: 7.0,
                plugin_latency_ms: 10.0,
            })
        );
    }
}
