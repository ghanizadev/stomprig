use std::time::{Duration, Instant};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub(crate) struct CpuUsage {
    last_sample: Instant,
    last_process_time: Option<u64>,
    usage: Option<f32>,
    logical_processors: f32,
}

impl Default for CpuUsage {
    fn default() -> Self {
        Self {
            last_sample: Instant::now() - SAMPLE_INTERVAL,
            last_process_time: None,
            usage: None,
            logical_processors: std::thread::available_parallelism()
                .map(|count| count.get() as f32)
                .unwrap_or(1.0),
        }
    }
}

impl CpuUsage {
    pub(crate) fn sample(&mut self) -> Option<f32> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_sample);
        if elapsed < SAMPLE_INTERVAL {
            return self.usage;
        }
        self.last_sample = now;

        let process_time = process_cpu_time()?;
        let Some(previous_process_time) = self.last_process_time.replace(process_time) else {
            return self.usage;
        };
        let process_delta = process_time.saturating_sub(previous_process_time) as f64;
        let wall_delta = elapsed.as_nanos() as f64;
        let usage = (process_delta * 100.0 / wall_delta * 100.0
            / f64::from(self.logical_processors))
        .clamp(0.0, 100.0) as f32;
        self.usage = Some(usage);
        self.usage
    }
}

#[cfg(windows)]
fn process_cpu_time() -> Option<u64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let success = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if success == 0 {
        return None;
    }

    Some(filetime_to_u64(kernel) + filetime_to_u64(user))
}

#[cfg(windows)]
fn filetime_to_u64(filetime: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(filetime.dwHighDateTime) << 32) | u64::from(filetime.dwLowDateTime)
}

#[cfg(not(windows))]
fn process_cpu_time() -> Option<u64> {
    None
}
