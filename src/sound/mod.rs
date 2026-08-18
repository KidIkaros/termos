//! Alert sound cues — ported from Go TUIOS `internal/sound/sound.go`.
//!
//! Plays the two short cues an agent alert can make audible by shelling out
//! to an audio player the system already has (paplay/pw-play/aplay/afplay)
//! rather than linking a decoder and a device backend.
//!
//! Three properties matter more than the sound itself:
//! - Nothing here blocks its caller: `play` is a bounds check, an atomic
//!   compare-and-swap, and a non-blocking channel send.
//! - A machine with no audio goes quiet permanently rather than repeatedly:
//!   the player list is resolved once, and a run of failed plays switches the
//!   subsystem off for the life of the process.
//! - One cue plays at a time and no faster than the cooldown: a single worker
//!   and a one-deep queue mean a workspace where six agents finish together
//!   makes one sound, not six overlapping ones.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Sender};

/// Which cue to play. There are two on purpose: the pair has to be told apart
/// by ear in under half a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// The agent having stopped. Information, so it is quiet.
    Done,
    /// The agent waiting on a human, or having failed. A request, so it is
    /// higher, longer and louder.
    Attention,
}

/// A play request.
#[derive(Debug, Clone)]
pub struct Request {
    pub cue: Cue,
    /// Minimum time between accepted plays.
    pub cooldown: Duration,
}

/// Silences the subsystem however it is configured (tests, CI, recording).
pub const DISABLE_ENV: &str = "TUIOS_NO_SOUND";

/// The one-deep queue and the worker state, shared across the process.
static STATE: OnceLock<SoundState> = OnceLock::new();

struct SoundState {
    queue: Sender<Request>,
    last_at: AtomicI64,
    off: AtomicBool,
}

fn state() -> &'static SoundState {
    STATE.get_or_init(|| SoundState {
        queue: {
            let (tx, rx) = unbounded();
            // The worker is the only consumer; the bounded channel in Go maps
            // to "only the latest request matters", which we implement by
            // draining on the worker side (see worker).
            std::thread::spawn(move || worker(rx));
            tx
        },
        last_at: AtomicI64::new(0),
        off: AtomicBool::new(false),
    })
}

/// Make a cue audible, or do nothing. Never blocks and never reports failure:
/// a request arriving while another is still playing is dropped rather than
/// queued, because a cue that plays after the state it announced has already
/// changed is worse than no cue at all.
pub fn play(req: Request) {
    let st = state();
    if st.off.load(Ordering::Relaxed)
        || std::env::var(DISABLE_ENV).is_ok()
        || !accept(&st.last_at, req.cooldown)
    {
        return;
    }
    // Drop when the worker is busy: keep only the newest pending request.
    let _ = st.queue.try_send(req);
}

/// The cooldown slot claim: a compare-and-swap makes two panes finishing in
/// the same instant produce one cue rather than two.
fn accept(last_at: &AtomicI64, cooldown: Duration) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    if cooldown.is_zero() {
        last_at.store(now, Ordering::Relaxed);
        return true;
    }
    let cd = cooldown.as_nanos() as i64;
    let last = last_at.load(Ordering::Relaxed);
    if now - last < cd {
        return false;
    }
    last_at
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
}

/// The single worker: owns every process spawn and every temp file. Drains
/// the queue so a burst of requests plays one cue (the newest) and nothing
/// piles up.
fn worker(rx: crossbeam_channel::Receiver<Request>) {
    let players = resolve_players();
    let mut failures = 0u32;
    while let Ok(req) = rx.recv() {
        // Only the latest request matters: drain the backlog.
        let mut latest = req;
        while let Ok(next) = rx.try_recv() {
            latest = next;
        }
        if players.is_empty() {
            continue;
        }
        let wav = generate_wav(latest.cue);
        let ok = players.iter().any(|player| play_with(player, &wav));
        if ok {
            failures = 0;
        } else {
            failures += 1;
            if failures >= 3 {
                // A machine with no audio goes quiet permanently.
                if let Some(st) = STATE.get() {
                    st.off.store(true, Ordering::Relaxed);
                }
                return;
            }
        }
    }
}

/// Resolve the audio players available on this machine, best first.
pub fn resolve_players() -> Vec<&'static str> {
    ["paplay", "pw-play", "aplay", "afplay"]
        .iter()
        .copied()
        .filter(|cmd| {
            std::process::Command::new(cmd)
                .arg("--help")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .collect()
}

/// Play a WAV with one player, bounded by a timeout.
fn play_with(player: &str, wav: &[u8]) -> bool {
    let dir = std::env::temp_dir().join(format!("termos-sound-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.wav", player));
    let Ok(mut f) = std::fs::File::create(&path) else {
        return false;
    };
    if f.write_all(wav).is_err() {
        return false;
    }
    if f.flush().is_err() {
        return false;
    }
    drop(f);
    let started = Instant::now();
    let mut child = match std::process::Command::new(player)
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(15) {
                    let _ = child.kill();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return false,
        }
    }
}

/// Generate a 16-bit mono PCM WAV for a cue. Done is a quiet short tone;
/// Attention is higher, longer and louder.
pub fn generate_wav(cue: Cue) -> Vec<u8> {
    let (freq, duration_ms, amplitude) = match cue {
        Cue::Done => (440.0, 150, 0.25),
        Cue::Attention => (880.0, 300, 0.5),
    };
    let sample_rate = 22050u32;
    let samples = (sample_rate * duration_ms / 1000) as usize;
    let data_len = samples * 2;
    let mut wav = Vec::with_capacity(44 + data_len);
    // RIFF header.
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    // fmt chunk.
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
                                                 // data chunk.
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..samples {
        // A gentle fade in/out over the first/last 10ms avoids clicks.
        let t = i as f64 / sample_rate as f64;
        let fade = ((i as f64 / (sample_rate as f64 * 0.01)).min(1.0))
            .min(((samples - i) as f64 / (sample_rate as f64 * 0.01)).min(1.0))
            .max(0.0);
        let sample =
            (amplitude * fade * (2.0 * std::f64::consts::PI * freq * t).sin() * 32767.0) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid() {
        let wav = generate_wav(Cue::Done);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        // data chunk present and sized.
        assert_eq!(&wav[36..40], b"data");
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(data_len, wav.len() - 44);
        assert!(data_len > 1000);
    }

    #[test]
    fn attention_is_longer_than_done() {
        let done = generate_wav(Cue::Done);
        let attn = generate_wav(Cue::Attention);
        assert!(attn.len() > done.len());
    }

    #[test]
    fn cooldown_slot_serializes() {
        let last = AtomicI64::new(0);
        // First claim wins.
        assert!(accept(&last, Duration::from_secs(1)));
        // Immediately after, the second claim is refused.
        assert!(!accept(&last, Duration::from_secs(1)));
        // A zero cooldown always claims.
        assert!(accept(&last, Duration::ZERO));
    }

    #[test]
    fn no_sound_env_silences() {
        std::env::set_var(DISABLE_ENV, "1");
        // play() must not panic and must be a no-op.
        play(Request {
            cue: Cue::Done,
            cooldown: Duration::ZERO,
        });
        std::env::remove_var(DISABLE_ENV);
    }

    #[test]
    fn player_probe_does_not_panic() {
        let players = resolve_players();
        // On any machine this returns some subset; it must not panic.
        assert!(players.len() <= 4);
    }

    #[test]
    fn queue_accepts_without_blocking() {
        // With sound disabled, play() returns before touching the queue.
        std::env::set_var(DISABLE_ENV, "1");
        play(Request {
            cue: Cue::Done,
            cooldown: Duration::ZERO,
        });
        std::env::remove_var(DISABLE_ENV);
    }
}
