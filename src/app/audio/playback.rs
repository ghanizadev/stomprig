use rodio::{Decoder, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Decoded audio owned by the plugin runtime while it is being played.
/// Samples are interleaved and normalized to f32 by rodio's decoder.
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
}

impl AudioClip {
    pub fn frames(&self) -> u64 {
        (self.samples.len() / self.channels.max(1)) as u64
    }
}

pub fn decode_file(path: &Path) -> Result<AudioClip, String> {
    let file = File::open(path).map_err(|error| format!("Could not open audio file: {error}"))?;
    let decoder = Decoder::try_from(BufReader::new(file))
        .map_err(|error| format!("Could not decode audio file: {error}"))?;
    let channels = usize::from(decoder.channels().get()).max(1);
    let sample_rate = decoder.sample_rate().get();
    let mut samples: Vec<f32> = decoder.collect();
    let complete_sample_count = samples.len() / channels * channels;
    samples.truncate(complete_sample_count);
    if samples.is_empty() {
        return Err("The selected audio file contains no samples".to_owned());
    }
    Ok(AudioClip {
        samples,
        channels,
        sample_rate,
    })
}

pub struct PlaybackState {
    playing: AtomicBool,
    volume_bits: AtomicU32,
    position_frames: AtomicU64,
    seek_frame: AtomicU64,
    seek_pending: AtomicBool,
}

impl PlaybackState {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            position_frames: AtomicU64::new(0),
            seek_frame: AtomicU64::new(0),
            seek_pending: AtomicBool::new(false),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Release);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Acquire))
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume_bits
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }

    pub fn position_frames(&self) -> u64 {
        self.position_frames.load(Ordering::Acquire)
    }

    pub fn request_seek(&self, frame: u64) {
        self.seek_frame.store(frame, Ordering::Release);
        self.seek_pending.store(true, Ordering::Release);
    }

    pub fn reset(&self) {
        self.playing.store(false, Ordering::Release);
        self.position_frames.store(0, Ordering::Release);
        self.seek_frame.store(0, Ordering::Release);
        self.seek_pending.store(true, Ordering::Release);
    }

    pub(crate) fn take_seek(&self) -> Option<u64> {
        if self.seek_pending.swap(false, Ordering::AcqRel) {
            Some(self.seek_frame.load(Ordering::Acquire))
        } else {
            None
        }
    }

    pub(crate) fn set_position(&self, frame: u64) {
        self.position_frames.store(frame, Ordering::Release);
    }

    pub(crate) fn stop_at_end(&self) {
        self.playing.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::PlaybackState;

    #[test]
    fn transport_controls_update_shared_state() {
        let state = PlaybackState::new();
        assert!(!state.is_playing());
        assert_eq!(state.volume(), 1.0);

        state.set_volume(2.0);
        state.set_playing(true);
        state.request_seek(1234);

        assert!(state.is_playing());
        assert_eq!(state.volume(), 1.0);
        assert_eq!(state.take_seek(), Some(1234));
        assert_eq!(state.take_seek(), None);

        state.reset();
        assert!(!state.is_playing());
        assert_eq!(state.position_frames(), 0);
        assert_eq!(state.take_seek(), Some(0));
    }
}
