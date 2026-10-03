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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioBackend {
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
    pub routing: ChannelRouting,
}
