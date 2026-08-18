//! Event hub — ported from Go TUIOS `internal/session/daemon_events.go` and
//! `state_events.go`.
//!
//! Thread-safe pub/sub system for the session daemon. Event sources publish
//! typed events to the hub, which stamps each with a monotonic sequence
//! number and delivers it to every matching subscriber.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crossbeam_channel::{unbounded, Receiver, Sender};

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// Event type discriminators. These are part of the public protocol surface;
/// keep the string values stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    WindowCreated,
    WindowClosed,
    WindowExit,
    WindowRetitled,
    WindowFocused,
    WindowMoved,
    WindowMinimized,
    WindowRestored,
    WorkspaceSwitched,
    Output,
    Bell,
    ModeChanged,
    SessionCreated,
    SessionClosed,
    AgentStateChanged,
    Gap,
}

impl EventType {
    /// Convert to the wire string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WindowCreated => "window-created",
            Self::WindowClosed => "window-closed",
            Self::WindowExit => "window-exit",
            Self::WindowRetitled => "window-retitled",
            Self::WindowFocused => "window-focused",
            Self::WindowMoved => "window-moved",
            Self::WindowMinimized => "window-minimized",
            Self::WindowRestored => "window-restored",
            Self::WorkspaceSwitched => "workspace-switched",
            Self::Output => "output",
            Self::Bell => "bell",
            Self::ModeChanged => "mode-changed",
            Self::SessionCreated => "session-created",
            Self::SessionClosed => "session-closed",
            Self::AgentStateChanged => "agent-state-changed",
            Self::Gap => "gap",
        }
    }
}

/// A single event delivered to subscribers.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: u64,
    pub event_type: EventType,
    pub session: String,
    pub window: String,
    pub data: String,
    pub time: i64,
}

impl Event {
    /// Create a new event with zero seq (the hub stamps it on broadcast).
    pub fn new(event_type: EventType, session: &str, window: &str) -> Self {
        Self {
            seq: 0,
            event_type,
            session: session.to_string(),
            window: window.to_string(),
            data: String::new(),
            time: 0,
        }
    }

    /// Attach payload data.
    pub fn with_data(mut self, data: &str) -> Self {
        self.data = data.to_string();
        self
    }
}

/// Filter for selecting which events a subscriber receives.
/// A zero/empty filter matches everything.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub session: Option<String>,
    pub window: Option<String>,
    pub event_types: Option<Vec<EventType>>,
}

impl EventFilter {
    /// Create a filter that matches everything.
    pub fn all() -> Self {
        Self::default()
    }

    /// Filter by session name.
    pub fn session(name: &str) -> Self {
        Self {
            session: Some(name.to_string()),
            ..Default::default()
        }
    }

    /// Filter by session and window.
    pub fn session_window(session: &str, window: &str) -> Self {
        Self {
            session: Some(session.to_string()),
            window: Some(window.to_string()),
            ..Default::default()
        }
    }

    /// Filter by event types.
    pub fn types(types: Vec<EventType>) -> Self {
        Self {
            event_types: Some(types),
            ..Default::default()
        }
    }

    /// Check if an event matches this filter.
    pub fn matches(&self, event: &Event) -> bool {
        if let Some(ref s) = self.session {
            if event.session != *s {
                return false;
            }
        }
        if let Some(ref w) = self.window {
            if event.window != *w {
                return false;
            }
        }
        if let Some(ref types) = self.event_types {
            if !types.contains(&event.event_type) {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// EventHub
// ---------------------------------------------------------------------------

/// Default per-subscriber queue capacity.
#[allow(dead_code)]
const DEFAULT_QUEUE: usize = 256;

struct Subscription {
    filter: EventFilter,
    sender: Sender<Event>,
}

/// Thread-safe pub/sub event hub.
pub struct EventHub {
    next_id: AtomicU64,
    next_seq: AtomicU64,
    subs: Mutex<HashMap<u64, Subscription>>,
}

impl EventHub {
    /// Create a new event hub.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            next_seq: AtomicU64::new(1),
            subs: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribe to events matching the given filter.
    /// Returns (subscription_id, receiver).
    pub fn subscribe(&self, filter: EventFilter) -> (u64, Receiver<Event>) {
        let (tx, rx) = unbounded();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subs
            .lock()
            .unwrap()
            .insert(id, Subscription { filter, sender: tx });
        (id, rx)
    }

    /// Unsubscribe by ID.
    pub fn unsubscribe(&self, id: u64) {
        self.subs.lock().unwrap().remove(&id);
    }

    /// Broadcast an event to all matching subscribers.
    /// The event is stamped with a monotonic sequence number.
    pub fn broadcast(&self, mut event: Event) {
        event.seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let subs = self.subs.lock().unwrap();
        for sub in subs.values() {
            if sub.filter.matches(&event) {
                // Send drops events for slow subscribers rather than blocking.
                let _ = sub.sender.send(event.clone());
            }
        }
    }

    /// Current subscriber count.
    pub fn subscriber_count(&self) -> usize {
        self.subs.lock().unwrap().len()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// StateChange — for state_merge
// ---------------------------------------------------------------------------

/// A state change event for three-way merge: the before and after values,
/// plus the source that produced the change.
#[derive(Debug, Clone)]
pub struct StateChange {
    pub field: String,
    pub before: String,
    pub after: String,
    pub source: String,
}

impl StateChange {
    /// Create a new state change.
    pub fn new(field: &str, before: &str, after: &str, source: &str) -> Self {
        Self {
            field: field.to_string(),
            before: before.to_string(),
            after: after.to_string(),
            source: source.to_string(),
        }
    }

    /// Whether the field actually changed.
    pub fn changed(&self) -> bool {
        self.before != self.after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_and_broadcast() {
        let hub = EventHub::new();
        let (id, rx) = hub.subscribe(EventFilter::all());
        hub.broadcast(Event::new(EventType::WindowCreated, "s1", "w1"));
        let event = rx.recv().unwrap();
        assert_eq!(event.event_type, EventType::WindowCreated);
        assert_eq!(event.session, "s1");
        assert_eq!(event.window, "w1");
        assert!(event.seq > 0);
        hub.unsubscribe(id);
        assert_eq!(hub.subscriber_count(), 0);
    }

    #[test]
    fn filter_by_session() {
        let hub = EventHub::new();
        let (_id, rx) = hub.subscribe(EventFilter::session("s1"));
        hub.broadcast(Event::new(EventType::WindowCreated, "s1", "w1"));
        hub.broadcast(Event::new(EventType::WindowCreated, "s2", "w2"));
        let event = rx.recv().unwrap();
        assert_eq!(event.session, "s1");
        // s2 event should not arrive
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn filter_by_window() {
        let hub = EventHub::new();
        let (_id, rx) = hub.subscribe(EventFilter::session_window("s1", "w1"));
        hub.broadcast(Event::new(EventType::Output, "s1", "w1"));
        hub.broadcast(Event::new(EventType::Output, "s1", "w2"));
        let event = rx.recv().unwrap();
        assert_eq!(event.window, "w1");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn filter_by_type() {
        let hub = EventHub::new();
        let (_id, rx) = hub.subscribe(EventFilter::types(vec![EventType::Bell, EventType::Output]));
        hub.broadcast(Event::new(EventType::Bell, "s1", "w1"));
        hub.broadcast(Event::new(EventType::WindowClosed, "s1", "w1"));
        let event = rx.recv().unwrap();
        assert_eq!(event.event_type, EventType::Bell);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn multiple_subscribers() {
        let hub = EventHub::new();
        let (_id1, rx1) = hub.subscribe(EventFilter::all());
        let (_id2, rx2) = hub.subscribe(EventFilter::all());
        hub.broadcast(Event::new(EventType::Output, "s1", "w1"));
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
        assert_eq!(hub.subscriber_count(), 2);
    }

    #[test]
    fn unsubscribe_stops_delivery() {
        let hub = EventHub::new();
        let (id, rx) = hub.subscribe(EventFilter::all());
        hub.unsubscribe(id);
        hub.broadcast(Event::new(EventType::Output, "s1", "w1"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn seq_is_monotonic() {
        let hub = EventHub::new();
        let (_id, rx) = hub.subscribe(EventFilter::all());
        hub.broadcast(Event::new(EventType::Output, "s1", "w1"));
        hub.broadcast(Event::new(EventType::Output, "s1", "w1"));
        let e1 = rx.recv().unwrap();
        let e2 = rx.recv().unwrap();
        assert!(e2.seq > e1.seq);
    }

    #[test]
    fn state_change_detects_difference() {
        let c = StateChange::new("title", "old", "new", "user");
        assert!(c.changed());
        let c2 = StateChange::new("title", "same", "same", "user");
        assert!(!c2.changed());
    }

    #[test]
    fn event_with_data() {
        let e = Event::new(EventType::Output, "s1", "w1").with_data("hello");
        assert_eq!(e.data, "hello");
    }
}
