use super::types::{AudioBackend, DeviceInfo};
use cpal::traits::{DeviceTrait, HostTrait};

pub struct AudioDevices;

impl AudioDevices {
    pub fn available_backends() -> Vec<AudioBackend> {
        let mut result = vec![AudioBackend::Wasapi];
        if cpal::available_hosts().contains(&cpal::HostId::Asio) {
            result.push(AudioBackend::Asio);
        }
        result
    }
    fn host(backend: AudioBackend) -> Result<cpal::Host, String> {
        let id = match backend {
            AudioBackend::Wasapi => cpal::HostId::Wasapi,
            AudioBackend::Asio => cpal::HostId::Asio,
        };
        cpal::host_from_id(id).map_err(|e| format!("Could not open {} host: {e}", backend.label()))
    }
    pub fn input_devices(backend: AudioBackend) -> Result<Vec<DeviceInfo>, String> {
        let host = Self::host(backend)?;
        host.input_devices()
            .map_err(|e| format!("Could not enumerate input devices: {e}"))?
            .enumerate()
            .map(|(index, device)| {
                let name = device.to_string();
                let channels = device
                    .default_input_config()
                    .map_err(|e| format!("Could not read input configuration for {name}: {e}"))?
                    .channels();
                Ok(DeviceInfo {
                    index,
                    name,
                    channels: usize::from(channels),
                })
            })
            .collect()
    }
    pub fn output_devices(backend: AudioBackend) -> Result<Vec<DeviceInfo>, String> {
        let host = Self::host(backend)?;
        host.output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .enumerate()
            .map(|(index, device)| {
                let name = device.to_string();
                let channels = device
                    .default_output_config()
                    .map_err(|e| format!("Could not read output configuration for {name}: {e}"))?
                    .channels();
                Ok(DeviceInfo {
                    index,
                    name,
                    channels: usize::from(channels),
                })
            })
            .collect()
    }
    pub fn default_device_indices(backend: AudioBackend) -> (Option<usize>, Option<usize>) {
        let Ok(host) = Self::host(backend) else {
            return (None, None);
        };
        let input_name = host.default_input_device().map(|device| device.to_string());
        let output_name = host
            .default_output_device()
            .map(|device| device.to_string());
        let input = input_name.and_then(|name| {
            host.input_devices()
                .ok()?
                .enumerate()
                .find_map(|(i, d)| (d.to_string() == name).then_some(i))
        });
        let output = output_name.and_then(|name| {
            host.output_devices()
                .ok()?
                .enumerate()
                .find_map(|(i, d)| (d.to_string() == name).then_some(i))
        });
        (input, output)
    }
    pub fn sample_rate(backend: AudioBackend, output_index: usize) -> Result<f64, String> {
        let host = Self::host(backend)?;
        let device = host
            .output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .nth(output_index)
            .ok_or_else(|| format!("Output device {output_index} is unavailable"))?;
        Ok(device
            .default_output_config()
            .map_err(|e| format!("Could not read output configuration: {e}"))?
            .sample_rate() as f64)
    }
    pub(crate) fn selected_devices(
        backend: AudioBackend,
        input_index: usize,
        output_index: usize,
    ) -> Result<(cpal::Device, cpal::Device), String> {
        let host = Self::host(backend)?;
        let input = host
            .input_devices()
            .map_err(|e| format!("Could not enumerate input devices: {e}"))?
            .nth(input_index)
            .ok_or_else(|| format!("Input device {input_index} is unavailable"))?;
        let output = host
            .output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .nth(output_index)
            .ok_or_else(|| format!("Output device {output_index} is unavailable"))?;
        Ok((input, output))
    }
}
