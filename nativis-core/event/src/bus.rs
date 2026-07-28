use crate::EngineEvent;
use parking_lot::Mutex;
use std::sync::Arc;

/// Type alias for event listener callbacks.
pub type EventListener = Arc<dyn Fn(&EngineEvent) + Send + Sync + 'static>;

/// Thread-safe, synchronous pub-sub event bus.
///
/// `publish()` dispatches *immediately* to all registered listeners on the
/// calling thread. This keeps ordering predictable within a frame phase.
#[derive(Default, Clone)]
pub struct EventBus {
    listeners: Arc<Mutex<Vec<EventListener>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self { listeners: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Subscribe to *all* engine events. The callback receives a shared
    /// reference to each event in the order it was published.
    pub fn subscribe<F>(&self, listener: F)
    where
        F: Fn(&EngineEvent) + Send + Sync + 'static,
    {
        self.listeners.lock().push(Arc::new(listener));
    }

    /// Publish an event, dispatching immediately to every registered listener.
    pub fn publish(&self, event: EngineEvent) {
        let listeners = self.listeners.lock();
        for l in listeners.iter() {
            l(&event);
        }
    }

    /// Return the current number of registered listeners (useful for tests).
    pub fn listener_count(&self) -> usize {
        self.listeners.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn publish_reaches_subscribers() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        bus.subscribe(move |_| { c.fetch_add(1, Ordering::Relaxed); });
        bus.publish(EngineEvent::EngineStarted);
        bus.publish(EngineEvent::EngineShutdown);
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
