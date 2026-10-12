use super::types::{AudioBackend, DeviceInfo};
use cpal::traits::{DeviceTrait, HostTrait};
use std::str::FromStr;

pub struct AudioDevices;

impl AudioDevices {
    pub fn available_backends() -> Vec<AudioBackend> {
        cpal::available_hosts()
            .into_iter()
            .filter_map(AudioBackend::from_host_id)
            .collect()
    }

    pub fn default_backend() -> AudioBackend {
        let available = Self::available_backends();
        AudioBackend::from_host_id(cpal::default_host().id())
            .filter(|backend| available.contains(backend))
            .or_else(|| available.first().copied())
            .unwrap_or_else(AudioBackend::fallback)
    }

    fn host(backend: AudioBackend) -> Result<cpal::Host, String> {
        let id = backend.host_id()?;
        cpal::host_from_id(id).map_err(|e| format!("Could not open {} host: {e}", backend.label()))
    }
    pub fn input_devices(backend: AudioBackend) -> Result<Vec<DeviceInfo>, String> {
        let host = Self::host(backend)?;
        Ok(host
            .input_devices()
            .map_err(|e| format!("Could not enumerate input devices: {e}"))?
            .filter_map(|device| {
                device_info(&device, "input", |device| device.default_input_config())
            })
            .collect())
    }
    pub fn output_devices(backend: AudioBackend) -> Result<Vec<DeviceInfo>, String> {
        let host = Self::host(backend)?;
        Ok(host
            .output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .filter_map(|device| {
                device_info(&device, "output", |device| device.default_output_config())
            })
            .collect())
    }
    pub fn default_device_ids(backend: AudioBackend) -> (Option<String>, Option<String>) {
        let Ok(host) = Self::host(backend) else {
            return (None, None);
        };
        let input = host
            .default_input_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        let output = host
            .default_output_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        (input, output)
    }
    pub fn sample_rate(backend: AudioBackend, output_device_id: &str) -> Result<f64, String> {
        let host = Self::host(backend)?;
        let id = parse_device_id(output_device_id, "output")?;
        let device = host
            .output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .find(|device| device.id().is_ok_and(|device_id| device_id == id))
            .ok_or_else(|| format!("Output device {output_device_id} is unavailable"))?;
        Ok(device
            .default_output_config()
            .map_err(|e| format!("Could not read output configuration: {e}"))?
            .sample_rate() as f64)
    }
    pub(crate) fn selected_devices(
        backend: AudioBackend,
        input_device_id: &str,
        output_device_id: &str,
    ) -> Result<(cpal::Device, cpal::Device), String> {
        let host = Self::host(backend)?;
        let input_id = parse_device_id(input_device_id, "input")?;
        let output_id = parse_device_id(output_device_id, "output")?;
        let input = host
            .input_devices()
            .map_err(|e| format!("Could not enumerate input devices: {e}"))?
            .find(|device| device.id().is_ok_and(|device_id| device_id == input_id))
            .ok_or_else(|| format!("Input device {input_device_id} is unavailable"))?;
        let output = host
            .output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .find(|device| device.id().is_ok_and(|device_id| device_id == output_id))
            .ok_or_else(|| format!("Output device {output_device_id} is unavailable"))?;
        Ok((input, output))
    }
}

fn device_info(
    device: &cpal::Device,
    direction: &str,
    default_config: impl FnOnce(&cpal::Device) -> Result<cpal::SupportedStreamConfig, cpal::Error>,
) -> Option<DeviceInfo> {
    let name = device.to_string();
    let id = match device.id() {
        Ok(id) => id.to_string(),
        Err(error) => {
            log::debug!("skipping {direction} device {name:?}: could not read device ID: {error}");
            return None;
        }
    };
    let configuration = match default_config(device) {
        Ok(configuration) => configuration,
        Err(error) => {
            log::debug!(
                "skipping {direction} device {name:?} ({id}): could not read default configuration: {error}"
            );
            return None;
        }
    };
    Some(DeviceInfo {
        id,
        name,
        channels: usize::from(configuration.channels()),
    })
}

fn parse_device_id(device_id: &str, direction: &str) -> Result<cpal::DeviceId, String> {
    cpal::DeviceId::from_str(device_id)
        .map_err(|error| format!("Could not parse {direction} device ID: {error}"))
}
