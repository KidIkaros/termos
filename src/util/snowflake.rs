//! Snowflake-style 64-bit unique ID generator.
//!
//! IDs are structured as:
//! ```text
//! 0 | 41-bit timestamp | 5-bit worker | 5-bit datacenter | 12-bit sequence
//! ```
//!
//! - **Sign bit**: always 0 (positive).
//! - **Timestamp**: milliseconds since a custom epoch (2024-01-01 00:00:00 UTC).
//! - **Worker ID**: identifies the local process (0–31).
//! - **Datacenter ID**: identifies the daemon instance (0–31).
//! - **Sequence**: per-millisecond counter (0–4095), resets each millisecond.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Custom epoch: 2024-01-01 00:00:00 UTC in milliseconds since Unix epoch.
const EPOCH_MS: i64 = 1_704_067_200_000;

/// Bit positions.
const WORKER_BITS: u32 = 5;
const DATACENTER_BITS: u32 = 5;
const SEQUENCE_BITS: u32 = 12;

/// Masks.
const MAX_SEQUENCE: i64 = (1 << SEQUENCE_BITS) - 1;
const WORKER_SHIFT: u32 = SEQUENCE_BITS;
const DATACENTER_SHIFT: u32 = SEQUENCE_BITS + WORKER_BITS;
const TIMESTAMP_SHIFT: u32 = SEQUENCE_BITS + WORKER_BITS + DATACENTER_BITS;

/// A thread-safe Snowflake ID generator.
pub struct Snowflake {
    worker_id: i64,
    datacenter_id: i64,
    last_timestamp: AtomicI64,
    sequence: AtomicI64,
}

impl Snowflake {
    /// Create a new generator with the given worker and datacenter IDs.
    ///
    /// Both must be in the range 0–31 (5 bits each).
    pub fn new(worker_id: u8, datacenter_id: u8) -> Self {
        assert!(worker_id < 32, "worker_id must be 0–31");
        assert!(datacenter_id < 32, "datacenter_id must be 0–31");
        Self {
            worker_id: worker_id as i64,
            datacenter_id: datacenter_id as i64,
            last_timestamp: AtomicI64::new(-1),
            sequence: AtomicI64::new(0),
        }
    }

    /// Create a generator using process ID for worker identification.
    pub fn from_process() -> Self {
        let pid = std::process::id();
        let worker = (pid & 0x1F) as u8; // lower 5 bits of PID
        Self::new(worker, 0)
    }

    /// Generate the next unique ID.
    pub fn next_id(&self) -> i64 {
        let mut timestamp = Self::current_millis();
        let mut seq = self.sequence.load(Ordering::Relaxed);
        let last_ts = self.last_timestamp.load(Ordering::Relaxed);

        if timestamp == last_ts {
            // Same millisecond — increment sequence
            seq = (seq + 1) & MAX_SEQUENCE;
            if seq == 0 {
                // Sequence overflow — wait for next millisecond
                timestamp = Self::wait_next_millis(last_ts);
            }
        } else {
            // New millisecond — reset sequence
            seq = 0;
        }

        self.last_timestamp.store(timestamp, Ordering::Relaxed);
        self.sequence.store(seq, Ordering::Relaxed);

        ((timestamp - EPOCH_MS) << TIMESTAMP_SHIFT)
            | (self.datacenter_id << DATACENTER_SHIFT)
            | (self.worker_id << WORKER_SHIFT)
            | seq
    }

    /// Extract the timestamp from an ID.
    pub fn timestamp(id: i64) -> i64 {
        (id >> TIMESTAMP_SHIFT) + EPOCH_MS
    }

    /// Extract the worker ID from an ID.
    pub fn worker_id(id: i64) -> i8 {
        ((id >> WORKER_SHIFT) & 0x1F) as i8
    }

    /// Extract the datacenter ID from an ID.
    pub fn datacenter_id(id: i64) -> i8 {
        ((id >> DATACENTER_SHIFT) & 0x1F) as i8
    }

    /// Extract the sequence number from an ID.
    pub fn sequence(id: i64) -> i16 {
        (id & MAX_SEQUENCE) as i16
    }

    fn current_millis() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    fn wait_next_millis(last_ts: i64) -> i64 {
        let mut ts = Self::current_millis();
        while ts <= last_ts {
            ts = Self::current_millis();
        }
        ts
    }
}

impl Default for Snowflake {
    fn default() -> Self {
        Self::from_process()
    }
}

/// Global ID generator instance.
static GLOBAL: std::sync::OnceLock<Snowflake> = std::sync::OnceLock::new();

/// Get the global Snowflake generator.
pub fn global() -> &'static Snowflake {
    GLOBAL.get_or_init(Snowflake::from_process)
}

/// Generate a globally unique ID.
pub fn next_id() -> i64 {
    global().next_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique() {
        let gen = Snowflake::new(1, 1);
        let mut seen = HashSet::new();
        for _ in 0..10_000 {
            let id = gen.next_id();
            assert!(seen.insert(id), "duplicate ID: {id}");
        }
    }

    #[test]
    fn ids_are_positive() {
        let gen = Snowflake::new(0, 0);
        for _ in 0..100 {
            assert!(gen.next_id() > 0);
        }
    }

    #[test]
    fn ids_are_time_ordered() {
        let gen = Snowflake::new(1, 1);
        let mut prev = gen.next_id();
        // Give the clock a chance to advance
        std::thread::sleep(std::time::Duration::from_millis(2));
        for _ in 0..100 {
            let id = gen.next_id();
            assert!(id > prev, "IDs not ordered: {id} <= {prev}");
            prev = id;
        }
    }

    #[test]
    fn extract_fields() {
        let gen = Snowflake::new(5, 3);
        let id = gen.next_id();
        assert_eq!(Snowflake::worker_id(id), 5);
        assert_eq!(Snowflake::datacenter_id(id), 3);
        assert!(Snowflake::sequence(id) >= 0);
        assert!(Snowflake::timestamp(id) > 0);
    }

    #[test]
    fn different_workers_produce_different_ids() {
        let gen1 = Snowflake::new(1, 0);
        let gen2 = Snowflake::new(2, 0);
        let id1 = gen1.next_id();
        let id2 = gen2.next_id();
        // Different workers should produce different IDs (unless sequence wraps)
        assert_ne!(id1 & !MAX_SEQUENCE, id2 & !MAX_SEQUENCE);
    }

    #[test]
    fn from_process_does_not_panic() {
        let _gen = Snowflake::from_process();
    }
}
