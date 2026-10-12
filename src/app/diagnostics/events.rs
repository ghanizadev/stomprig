//! Fixed-size incidents passed from background threads to the UI.

use rtrb::{Consumer, Producer, RingBuffer};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const EVENT_QUEUE_CAPACITY: usize = 128;
pub(crate) const SUBJECT_BYTES: usize = 64;
const MESSAGE_BYTES: usize = 160;
const MAX_INCIDENTS: usize = 200;
const COALESCE_INTERVAL_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FixedText<const N: usize> {
    bytes: [u8; N],
    len: u16,
}

impl<const N: usize> Default for FixedText<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> FixedText<N> {
    pub(crate) fn new(value: &str) -> Self {
        let mut text = Self::default();
        let mut len = value.len().min(N);
        while !value.is_char_boundary(len) {
            len -= 1;
        }
        text.bytes[..len].copy_from_slice(&value.as_bytes()[..len]);
        text.len = len as u16;
        text
    }

    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum DiagnosticSeverity {
    Warning,
    Error,
}

impl DiagnosticSeverity {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum DiagnosticKind {
    Startup,
    PreviousCrash,
    InputStream,
    OutputStream,
    PluginProcessing,
    RuntimeWorker,
    PluginRecovery,
}

impl DiagnosticKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Startup => "Startup",
            Self::PreviousCrash => "Previous crash",
            Self::InputStream => "Audio input",
            Self::OutputStream => "Audio output",
            Self::PluginProcessing => "Plug-in processing",
            Self::RuntimeWorker => "Runtime worker",
            Self::PluginRecovery => "Plug-in recovery",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticEvent {
    pub(crate) timestamp_ms: u64,
    pub(crate) severity: DiagnosticSeverity,
    pub(crate) kind: DiagnosticKind,
    pub(crate) subject: FixedText<SUBJECT_BYTES>,
    message: FixedText<MESSAGE_BYTES>,
}

impl DiagnosticEvent {
    pub(crate) fn new(
        severity: DiagnosticSeverity,
        kind: DiagnosticKind,
        subject: &str,
        message: &str,
    ) -> Self {
        Self {
            timestamp_ms: timestamp_ms(),
            severity,
            kind,
            subject: FixedText::new(subject),
            message: FixedText::new(message),
        }
    }

    pub(crate) fn message(&self) -> &str {
        self.message.as_str()
    }

    fn same_incident(&self, other: &Self) -> bool {
        self.severity == other.severity
            && self.kind == other.kind
            && self.subject == other.subject
            && self.message == other.message
    }
}

pub(crate) struct DiagnosticProducer {
    producer: Producer<DiagnosticEvent>,
    dropped: Arc<AtomicU64>,
}

impl DiagnosticProducer {
    pub(crate) fn send(&mut self, event: DiagnosticEvent) {
        if self.producer.push(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct DiagnosticConsumer {
    consumer: Consumer<DiagnosticEvent>,
    dropped: Arc<AtomicU64>,
}

impl DiagnosticConsumer {
    pub(crate) fn try_recv(&mut self) -> Option<DiagnosticEvent> {
        self.consumer.pop().ok()
    }

    pub(crate) fn take_dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::AcqRel)
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.consumer.is_abandoned() && self.consumer.slots() == 0
    }
}

pub(crate) fn channel(capacity: usize) -> (DiagnosticProducer, DiagnosticConsumer) {
    let (producer, consumer) = RingBuffer::new(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    (
        DiagnosticProducer {
            producer,
            dropped: Arc::clone(&dropped),
        },
        DiagnosticConsumer { consumer, dropped },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticIncident {
    pub(crate) event: DiagnosticEvent,
    pub(crate) occurrences: u64,
}

#[derive(Default)]
pub(crate) struct DiagnosticHistory {
    incidents: VecDeque<DiagnosticIncident>,
    dropped_events: u64,
}

impl DiagnosticHistory {
    pub(crate) fn record(&mut self, event: DiagnosticEvent) -> bool {
        if let Some(index) = self.incidents.iter().rposition(|incident| {
            incident.event.same_incident(&event)
                && event
                    .timestamp_ms
                    .saturating_sub(incident.event.timestamp_ms)
                    <= COALESCE_INTERVAL_MS
        }) {
            let mut incident = self
                .incidents
                .remove(index)
                .expect("diagnostic incident index was checked");
            incident.event = event;
            incident.occurrences = incident.occurrences.saturating_add(1);
            self.incidents.push_back(incident);
            return false;
        }

        if self.incidents.len() == MAX_INCIDENTS {
            self.incidents.pop_front();
        }
        self.incidents.push_back(DiagnosticIncident {
            event,
            occurrences: 1,
        });
        true
    }

    pub(crate) fn note_dropped(&mut self, count: u64) {
        self.dropped_events = self.dropped_events.saturating_add(count);
    }

    pub(crate) fn incidents(&self) -> &VecDeque<DiagnosticIncident> {
        &self.incidents
    }

    pub(crate) fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    pub(crate) fn len(&self) -> usize {
        self.incidents.len()
    }
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub(crate) fn format_timestamp_utc(timestamp_ms: u64) -> String {
    let seconds = timestamp_ms / 1_000;
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted_days = days_since_epoch + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: DiagnosticKind, subject: &str, message: &str) -> DiagnosticEvent {
        DiagnosticEvent::new(DiagnosticSeverity::Error, kind, subject, message)
    }

    #[test]
    fn bounded_queue_counts_overflow() {
        let (mut producer, mut consumer) = channel(1);
        let first = event(DiagnosticKind::InputStream, "Interface", "Disconnected");
        producer.send(first);
        producer.send(event(
            DiagnosticKind::OutputStream,
            "Speakers",
            "Unavailable",
        ));

        assert_eq!(consumer.try_recv(), Some(first));
        assert_eq!(consumer.try_recv(), None);
        assert_eq!(consumer.take_dropped(), 1);
    }

    #[test]
    fn history_coalesces_repeats_and_preserves_other_incidents() {
        let mut history = DiagnosticHistory::default();
        let repeated = event(
            DiagnosticKind::PluginProcessing,
            "Compressor",
            "Dry signal used",
        );
        history.record(repeated);
        history.record(event(
            DiagnosticKind::OutputStream,
            "Speakers",
            "Device unavailable",
        ));
        history.record(repeated);

        assert_eq!(history.len(), 2);
        let latest = history.incidents().back().unwrap();
        assert_eq!(latest.event.kind, DiagnosticKind::PluginProcessing);
        assert_eq!(latest.occurrences, 2);
    }

    #[test]
    fn fixed_text_truncates_only_at_utf8_boundaries() {
        let text = FixedText::<4>::new("ééé");
        assert_eq!(text.as_str(), "éé");
    }

    #[test]
    fn timestamp_formats_unix_epoch_and_a_leap_day() {
        assert_eq!(format_timestamp_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            format_timestamp_utc(1_709_164_800_000),
            "2024-02-29 00:00:00 UTC"
        );
    }
}
