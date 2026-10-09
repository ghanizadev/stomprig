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
        let usage = calculate_usage_percent(
            process_delta,
            wall_delta,
            f64::from(self.logical_processors),
        );
        self.usage = Some(usage);
        self.usage
    }
}

fn calculate_usage_percent(
    process_delta_nanos: f64,
    wall_delta_nanos: f64,
    processors: f64,
) -> f32 {
    (process_delta_nanos * 100.0 / wall_delta_nanos / processors).clamp(0.0, 100.0) as f32
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

    filetime_to_u64(kernel)
        .checked_add(filetime_to_u64(user))?
        .checked_mul(100)
}

#[cfg(windows)]
fn filetime_to_u64(filetime: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(filetime.dwHighDateTime) << 32) | u64::from(filetime.dwLowDateTime)
}

#[cfg(unix)]
fn process_cpu_time() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let success = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if success != 0 {
        return None;
    }

    let usage = unsafe { usage.assume_init() };
    timeval_to_nanos(usage.ru_utime)?.checked_add(timeval_to_nanos(usage.ru_stime)?)
}

#[cfg(unix)]
fn timeval_to_nanos(time: libc::timeval) -> Option<u64> {
    u64::try_from(time.tv_sec)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(time.tv_usec).ok()?.checked_mul(1_000)?)
}

#[cfg(not(any(unix, windows)))]
fn process_cpu_time() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::calculate_usage_percent;

    #[test]
    fn reports_one_busy_core_as_a_share_of_total_capacity() {
        let usage = calculate_usage_percent(500_000_000.0, 500_000_000.0, 8.0);

        assert_eq!(usage, 12.5);
    }

    #[test]
    fn caps_usage_when_all_processors_are_busy() {
        let usage = calculate_usage_percent(4_000_000_000.0, 500_000_000.0, 8.0);

        assert_eq!(usage, 100.0);
    }
}
