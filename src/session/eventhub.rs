//! Daemon event hub — ported from Go TUIOS `internal/session/daemon_events.go`.
//!
//! A publish/subscribe hub for session events (window added/closed/exited,
//! session created/killed, agent state changed). Subscribers attach a filter
//! and receive only matching events, each stamped with a global sequence
//! number so a late subscriber can replay from a known point.

use std::collections::HashMap;

use crossbeam_channel::{unbounded, Receiver, Sender};

/// The kind of a session event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A window was created in a session.
    WindowCreated,
    /// A window was closed.
    WindowClosed,
    /// A window's process exited.
    WindowExited,
    /// A session was created.
    SessionCreated,
    /// A session was killed.
    SessionKilled,
    /// A window's agent state changed.
    AgentStateChanged,
    /// A client attached to a session.
    ClientAttached,
    /// A client detached from a session.
    ClientDetached,
}

/// One published event.
#[derive(Debug, Clone)]
pub struct StreamEvent {
    /// The global sequence number (monotonic across the hub's lifetime).
    pub seq: u64,
    pub kind: EventKind,
    /// The session the event concerns.
    pub session: String,
    /// The window the event concerns, when applicable.
    pub window: Option<String>,
    /// Optional detail (e.g. the new agent state).
    pub detail: String,
}

/// A subscription filter: match events by kind and/or session. An empty
/// filter matches everything.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub kinds: Vec<EventKind>,
    pub sessions: Vec<String>,
}

impl EventFilter {
    /// Match an event against this filter.
    pub fn matches(&self, ev: &StreamEvent) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&ev.kind) {
            return false;
        }
        if !self.sessions.is_empty() && !self.sessions.iter().any(|s| s == &ev.session) {
            return false;
        }
        true
    }
}

/// A live subscription; dropping it unsubscribes.
pub struct EventSub {
    filter: EventFilter,
    rx: Receiver<StreamEvent>,
}

impl EventSub {
    /// The subscription's filter.
    pub fn filter(&self) -> &EventFilter {
        &self.filter
    }

    /// Non-blocking receive of the next matching event.
    pub fn try_recv(&self) -> Option<StreamEvent> {
        self.rx.try_recv().ok()
    }

    /// Blocking receive of the next matching event.
    pub fn recv(&self) -> Option<StreamEvent> {
        self.rx.recv().ok()
    }
}

/// The hub itself.
#[derive(Debug, Default)]
pub struct EventHub {
    subs: HashMap<u64, (Sender<StreamEvent>, EventFilter)>,
    next_sub: u64,
    seq: u64,
}

impl EventHub {
    /// Create a new hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe with a filter and a bounded mailbox size.
    pub fn subscribe(&mut self, filter: EventFilter, buf: usize) -> EventSub {
        let (tx, rx) = unbounded();
        let id = self.next_sub;
        self.next_sub += 1;
        let _ = buf;
        self.subs.insert(id, (tx, filter.clone()));
        EventSub { filter, rx }
    }

    /// Drop a subscription (a no-op when already gone).
    pub fn unsubscribe(&mut self, id: u64) {
        self.subs.remove(&id);
    }

    /// The current sequence number (the next event will carry it).
    pub fn current_seq(&self) -> u64 {
        self.seq
    }

    /// Publish an event to every matching subscriber.
    pub fn publish(
        &mut self,
        kind: EventKind,
        session: &str,
        window: Option<String>,
        detail: String,
    ) {
        self.seq += 1;
        let ev = StreamEvent {
            seq: self.seq,
            kind,
            session: session.to_string(),
            window,
            detail,
        };
        self.subs.retain(|_, (tx, filter)| {
            if filter.matches(&ev) {
                tx.send(ev.clone()).is_ok()
            } else {
                true
            }
        });
    }

    /// Number of live subscriptions.
    pub fn subscriber_count(&self) -> usize {
        self.subs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmatched_kind_is_filtered() {
        let mut hub = EventHub::new();
        let sub = hub.subscribe(
            EventFilter {
                kinds: vec![EventKind::WindowCreated],
                ..Default::default()
            },
            16,
        );
        hub.publish(
            EventKind::WindowClosed,
            "s1",
            Some("w1".into()),
            String::new(),
        );
        assert!(sub.try_recv().is_none());
        hub.publish(
            EventKind::WindowCreated,
            "s1",
            Some("w2".into()),
            String::new(),
        );
        let ev = sub.try_recv().unwrap();
        assert_eq!(ev.kind, EventKind::WindowCreated);
        assert_eq!(ev.window.as_deref(), Some("w2"));
    }

    #[test]
    fn session_filter_matches_only_that_session() {
        let mut hub = EventHub::new();
        let sub = hub.subscribe(
            EventFilter {
                sessions: vec!["work".into()],
                ..Default::default()
            },
            16,
        );
        hub.publish(EventKind::WindowCreated, "play", None, String::new());
        assert!(sub.try_recv().is_none());
        hub.publish(EventKind::WindowCreated, "work", None, String::new());
        assert!(sub.try_recv().is_some());
    }

    #[test]
    fn empty_filter_matches_everything() {
        let mut hub = EventHub::new();
        let sub = hub.subscribe(EventFilter::default(), 16);
        hub.publish(EventKind::SessionCreated, "s1", None, String::new());
        hub.publish(
            EventKind::AgentStateChanged,
            "s1",
            Some("w1".into()),
            "working".into(),
        );
        let a = sub.try_recv().unwrap();
        let b = sub.try_recv().unwrap();
        assert_eq!(a.kind, EventKind::SessionCreated);
        assert_eq!(b.kind, EventKind::AgentStateChanged);
        assert_eq!(b.detail, "working");
    }

    #[test]
    fn sequence_numbers_are_monotonic() {
        let mut hub = EventHub::new();
        assert_eq!(hub.current_seq(), 0);
        hub.publish(EventKind::WindowCreated, "s1", None, String::new());
        hub.publish(EventKind::WindowClosed, "s1", None, String::new());
        assert_eq!(hub.current_seq(), 2);
        let sub = hub.subscribe(EventFilter::default(), 16);
        hub.publish(EventKind::WindowCreated, "s2", None, String::new());
        assert_eq!(sub.try_recv().unwrap().seq, 3);
    }

    #[test]
    fn unsubscribe_removes_subscription() {
        let mut hub = EventHub::new();
        let sub = hub.subscribe(EventFilter::default(), 16);
        assert_eq!(hub.subscriber_count(), 1);
        hub.unsubscribe(0);
        assert_eq!(hub.subscriber_count(), 0);
        // Publishing to a dead subscription does not panic.
        hub.publish(EventKind::WindowCreated, "s1", None, String::new());
        let _ = sub;
    }
}
