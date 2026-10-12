use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) const PARAMETER_CHANGE_QUEUE_CAPACITY: usize = 1024;
const PARAMETER_FEEDBACK_QUEUE_CAPACITY: usize = 2048;

pub(crate) struct ParameterRuntimeFeedbackProducer {
    producer: Producer<(u32, f64)>,
    dropped_updates: Arc<AtomicUsize>,
}

pub(crate) struct ParameterRuntimeFeedbackConsumer {
    consumer: Consumer<(u32, f64)>,
    dropped_updates: Arc<AtomicUsize>,
}

pub(crate) struct ParameterChangeProducer {
    producer: Producer<(u32, f64)>,
}

pub(crate) struct ParameterChangeConsumer {
    consumer: Consumer<(u32, f64)>,
}

pub(crate) fn parameter_change_queue() -> (ParameterChangeProducer, ParameterChangeConsumer) {
    let (producer, consumer) = RingBuffer::new(PARAMETER_CHANGE_QUEUE_CAPACITY);

    (
        ParameterChangeProducer { producer },
        ParameterChangeConsumer { consumer },
    )
}

impl ParameterChangeProducer {
    pub(crate) fn push(&mut self, parameter_id: u32, value: f64) -> Result<(), String> {
        self.producer
            .push((parameter_id, value))
            .map_err(|_| "plugin parameter control queue is full".to_owned())
    }

    pub(crate) fn available_slots(&self) -> usize {
        self.producer.slots()
    }
}

impl ParameterChangeConsumer {
    pub(crate) fn pop(&mut self) -> Option<(u32, f64)> {
        self.consumer.pop().ok()
    }

    pub(crate) fn drain_into(&mut self, changes: &mut Vec<(u32, f64)>) {
        let pending = self.consumer.slots();
        for _ in 0..pending {
            if let Some(change) = self.pop() {
                changes.push(change);
            } else {
                break;
            }
        }
    }
}

pub(crate) fn parameter_runtime_feedback_queue() -> (
    ParameterRuntimeFeedbackProducer,
    ParameterRuntimeFeedbackConsumer,
) {
    let (producer, consumer) = RingBuffer::new(PARAMETER_FEEDBACK_QUEUE_CAPACITY);
    let dropped_updates = Arc::new(AtomicUsize::new(0));

    (
        ParameterRuntimeFeedbackProducer {
            producer,
            dropped_updates: Arc::clone(&dropped_updates),
        },
        ParameterRuntimeFeedbackConsumer {
            consumer,
            dropped_updates,
        },
    )
}

impl ParameterRuntimeFeedbackProducer {
    pub(crate) fn push(&mut self, parameter_id: u32, value: f64) {
        if self.producer.push((parameter_id, value)).is_err() {
            self.dropped_updates.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl ParameterRuntimeFeedbackConsumer {
    pub(crate) fn pop(&mut self) -> Option<(u32, f64)> {
        self.consumer.pop().ok()
    }

    pub(crate) fn take_dropped_updates(&self) -> usize {
        self.dropped_updates.swap(0, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PARAMETER_CHANGE_QUEUE_CAPACITY, parameter_change_queue, parameter_runtime_feedback_queue,
    };

    #[test]
    fn transfers_parameter_changes_without_sharing_the_plugin_lock() {
        let (mut producer, mut consumer) = parameter_change_queue();

        producer.push(42, 0.75).unwrap();

        assert_eq!(consumer.pop(), Some((42, 0.75)));
        assert_eq!(consumer.pop(), None);
    }

    #[test]
    fn rejects_updates_when_the_bounded_queue_is_full() {
        let (mut producer, mut consumer) = parameter_change_queue();
        for parameter_id in 0..PARAMETER_CHANGE_QUEUE_CAPACITY {
            producer.push(parameter_id as u32, 0.5).unwrap();
        }

        assert!(producer.push(42, 0.75).is_err());

        let mut changes = Vec::new();
        consumer.drain_into(&mut changes);
        assert_eq!(changes.len(), PARAMETER_CHANGE_QUEUE_CAPACITY);
    }

    #[test]
    fn returns_runtime_parameter_feedback_without_unbounded_storage() {
        let (mut producer, mut consumer) = parameter_runtime_feedback_queue();

        producer.push(17, 0.25);

        assert_eq!(consumer.pop(), Some((17, 0.25)));
        assert_eq!(consumer.pop(), None);
        assert_eq!(consumer.take_dropped_updates(), 0);
    }
}
