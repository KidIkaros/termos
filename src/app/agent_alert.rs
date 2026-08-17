//! Agent alert policy — ported from TUIOS `internal/app/agent_alert.go` and
//! `internal/config/agent_alerts.go`.
//!
//! Agent alerts run on the client, not the daemon, and that is the design
//! rather than an accident: the terminal an in-band notification has to reach,
//! the user config, and the dock all live only on the client. The client
//! alerts on the authoritative transitions it observes through the daemon's
//! `AgentStateChanged` broadcasts. A session with nobody attached raises
//! nothing.

use std::time::{Duration, Instant};

use crate::config::userconfig::{AgentAlertSounds, AgentAlertStates, AgentAlertsConfig};

/// Accepted agent-state wire values, in a stable order (the protocol surface).
pub const AGENT_STATE_NAMES: [&str; 6] =
    ["none", "working", "needs_input", "idle", "done", "errored"];

/// Validate an agent-state wire value (empty input is not accepted: the verb
/// requires a state, and "none" is the spelling that clears it).
pub fn parse_agent_state(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    AGENT_STATE_NAMES
        .iter()
        .find(|n| **n == s)
        .map(|n| n.to_string())
}

/// How an audible alert is made audible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundMode {
    /// Play a cue through a system audio player.
    Audio,
    /// Write a BEL and let the terminal decide.
    Bell,
    /// Do each.
    Both,
}

impl SoundMode {
    pub fn parse(s: &str) -> (SoundMode, bool) {
        match s.trim() {
            "" => (SoundMode::Audio, true),
            "audio" => (SoundMode::Audio, true),
            "bell" => (SoundMode::Bell, true),
            "both" => (SoundMode::Both, true),
            _ => (SoundMode::Audio, false),
        }
    }
}

/// `AgentAlertsConfig` with every default resolved and the quiet-hours string
/// parsed, so the hot path is field reads and integer comparisons.
#[derive(Debug, Clone)]
pub struct AgentAlertPolicy {
    pub enabled: bool,
    pub notify: bool,
    pub sound: bool,
    pub sound_mode: SoundMode,
    pub sound_cooldown: Duration,
    pub sound_done: String,
    pub sound_needs_input: String,
    pub dock: bool,
    pub suppress_focused: bool,
    pub settle: Duration,
    /// Which states alert (absent ⇒ no alert).
    states: [bool; 6],
    /// Minutes since local midnight; equal values mean no quiet window.
    quiet_from: i32,
    quiet_to: i32,
}

fn bool_or(v: Option<bool>, def: bool) -> bool {
    v.unwrap_or(def)
}

/// Resolve the config table into a policy, applying every default. A `None`
/// receiver resolves to the defaults, so a caller with no config at all still
/// gets the documented behavior.
pub fn resolve_agent_alerts(c: &AgentAlertsConfig) -> AgentAlertPolicy {
    let mut p = AgentAlertPolicy {
        enabled: true,
        notify: true,
        sound: false,
        sound_mode: SoundMode::Audio,
        sound_cooldown: Duration::from_secs(3),
        sound_done: String::new(),
        sound_needs_input: String::new(),
        dock: true,
        suppress_focused: true,
        settle: Duration::from_secs(2),
        states: [false; 6],
        quiet_from: 0,
        quiet_to: 0,
    };
    // Default states: needs_input/errored/done alert; idle/working do not.
    for (name, def) in [
        ("needs_input", true),
        ("errored", true),
        ("done", true),
        ("idle", false),
        ("working", false),
    ] {
        p.set_state(name, def);
    }

    p.enabled = bool_or(c.enabled, p.enabled);
    p.notify = bool_or(c.notify, p.notify);
    p.sound = bool_or(c.sound, p.sound);
    p.dock = bool_or(c.dock, p.dock);
    p.suppress_focused = bool_or(c.suppress_focused, p.suppress_focused);

    let AgentAlertStates {
        needs_input,
        errored,
        done,
        idle,
        working,
    } = &c.states;
    p.set_state(
        "needs_input",
        bool_or(*needs_input, p.alerts("needs_input")),
    );
    p.set_state("errored", bool_or(*errored, p.alerts("errored")));
    p.set_state("done", bool_or(*done, p.alerts("done")));
    p.set_state("idle", bool_or(*idle, p.alerts("idle")));
    p.set_state("working", bool_or(*working, p.alerts("working")));

    let (mode, ok) = SoundMode::parse(&c.sound_mode);
    if ok {
        p.sound_mode = mode;
    }
    if let Some(secs) = c.sound_cooldown_seconds {
        if secs >= 0 {
            p.sound_cooldown = Duration::from_secs(secs as u64);
        }
    }
    let AgentAlertSounds { done, needs_input } = &c.sounds;
    p.sound_done = done.trim().to_string();
    p.sound_needs_input = needs_input.trim().to_string();
    if let Some(secs) = c.settle_seconds {
        if secs >= 0 {
            p.settle = Duration::from_secs(secs as u64);
        }
    }
    if let Ok((from, to)) = parse_quiet_hours(&c.quiet_hours) {
        p.quiet_from = from;
        p.quiet_to = to;
    }
    p
}

impl AgentAlertPolicy {
    fn set_state(&mut self, name: &str, v: bool) {
        if let Some(i) = AGENT_STATE_NAMES.iter().position(|n| *n == name) {
            self.states[i] = v;
        }
    }

    /// Whether a transition into `state` is one the user asked to hear about.
    pub fn alerts(&self, state: &str) -> bool {
        self.enabled
            && AGENT_STATE_NAMES
                .iter()
                .position(|n| *n == state)
                .map(|i| self.states[i])
                .unwrap_or(false)
    }

    /// Whether an alert should play a cue through an audio player.
    pub fn plays_audio(&self) -> bool {
        self.sound && (self.sound_mode == SoundMode::Audio || self.sound_mode == SoundMode::Both)
    }

    /// Whether an alert should write a BEL to the terminal.
    pub fn plays_bell(&self) -> bool {
        self.sound && (self.sound_mode == SoundMode::Bell || self.sound_mode == SoundMode::Both)
    }

    /// Whether a transition into `state` uses the cue that asks for a human
    /// rather than the one that reports the machine stopped.
    pub fn attention_cue(&self, state: &str) -> bool {
        state == "needs_input" || state == "errored"
    }

    /// The user's replacement cue for a transition into `state`, or empty for
    /// the built-in one.
    pub fn cue_file(&self, state: &str) -> &str {
        if self.attention_cue(state) {
            &self.sound_needs_input
        } else {
            &self.sound_done
        }
    }

    /// Whether `now` falls inside the configured quiet-hours window (a window
    /// that wraps midnight is the union of the two halves).
    pub fn quiet(&self, now_minutes_since_midnight: i32) -> bool {
        if self.quiet_from == self.quiet_to {
            return false;
        }
        let m = now_minutes_since_midnight;
        if self.quiet_from < self.quiet_to {
            m >= self.quiet_from && m < self.quiet_to
        } else {
            m >= self.quiet_from || m < self.quiet_to
        }
    }
}

/// Parse `"HH:MM-HH:MM"` into minutes since local midnight. An empty string
/// is not an error and yields an empty window (0, 0), read as "never quiet".
pub fn parse_quiet_hours(s: &str) -> Result<(i32, i32), String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok((0, 0));
    }
    let (start, end) = s
        .split_once('-')
        .ok_or_else(|| format!("want HH:MM-HH:MM, got {s:?}"))?;
    let from = parse_clock(start)?;
    let to = parse_clock(end)?;
    Ok((from, to))
}

fn parse_clock(s: &str) -> Result<i32, String> {
    let (h, m) = s
        .trim()
        .split_once(':')
        .ok_or_else(|| format!("want HH:MM, got {s:?}"))?;
    let hours: i32 = h
        .trim()
        .parse()
        .map_err(|_| format!("want HH:MM, got {s:?}"))?;
    let mins: i32 = m
        .trim()
        .parse()
        .map_err(|_| format!("want HH:MM, got {s:?}"))?;
    if !(0..=23).contains(&hours) {
        return Err(format!("hour out of range in {s:?}"));
    }
    if !(0..=59).contains(&mins) {
        return Err(format!("minute out of range in {s:?}"));
    }
    Ok(hours * 60 + mins)
}

/// An alert waiting out the settle window. Holding the window id rather than
/// a pointer means a pane closed during the wait simply fails the lookup
/// instead of resurrecting a dead window.
#[derive(Debug, Clone)]
pub struct PendingAgentAlert {
    pub window_id: String,
    pub from: String,
    pub to: String,
    pub due: Instant,
}

/// The cue registry: one cooldown across every pane, so a workspace where six
/// agents finish together makes one sound rather than six.
#[derive(Debug)]
pub struct SoundCue {
    last_played: Option<Instant>,
}

impl SoundCue {
    pub fn new() -> Self {
        Self { last_played: None }
    }

    /// Play `file` (or a generated beep when empty) if the cooldown has
    /// elapsed. Returns whether it played.
    pub fn play(&mut self, file: &str, cooldown: Duration) -> bool {
        let now = Instant::now();
        if let Some(last) = self.last_played {
            if now.duration_since(last) < cooldown {
                return false;
            }
        }
        self.last_played = Some(now);
        play_cue(file);
        true
    }
}

impl Default for SoundCue {
    fn default() -> Self {
        Self::new()
    }
}

/// Play a cue file, or a synthesized beep when `file` is empty or missing.
/// Spawns the first available player and returns immediately; never blocks on
/// an audio device.
fn play_cue(file: &str) {
    let player = ["paplay", "aplay", "afplay", "play"]
        .iter()
        .find(|p| which(p));
    let mut cmd = match (player, file) {
        (Some(p), f) if !f.is_empty() && std::path::Path::new(f).exists() => {
            let mut c = std::process::Command::new(p);
            c.arg(f);
            c
        }
        (Some(p), _) => match write_beep_wav() {
            Some(path) => {
                let mut c = std::process::Command::new(p);
                c.arg(path.display().to_string());
                c
            }
            None => return,
        },
        (None, _) => {
            log::debug!("agent alert: no audio player found (paplay/aplay/afplay/play)");
            return;
        }
    };
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// Write a short 440 Hz beep WAV to a temp file. Kept tiny so the sound sink
/// works out of the box with no asset files. Returns the path.
fn write_beep_wav() -> Option<std::path::PathBuf> {
    let sample_rate = 22050u32;
    let seconds = 0.2;
    let n = (sample_rate as f64 * seconds) as usize;
    let mut data = Vec::with_capacity(44 + n * 2);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&((36 + n * 2) as u32).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes()); // PCM
    data.extend_from_slice(&1u16.to_le_bytes()); // mono
    data.extend_from_slice(&sample_rate.to_le_bytes());
    data.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    data.extend_from_slice(&2u16.to_le_bytes()); // block align
    data.extend_from_slice(&16u16.to_le_bytes()); // bits
    data.extend_from_slice(b"data");
    data.extend_from_slice(&((n * 2) as u32).to_le_bytes());
    let mut phase = 0.0f64;
    for _ in 0..n {
        let amp = 0.5 - 0.5 * (phase / seconds).cos(); // fade in/out
        let sample = (0.4 * amp * (2.0 * std::f64::consts::PI * 440.0 * phase).sin()) as f32;
        let v = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
        phase += 1.0 / sample_rate as f64;
    }
    let path = std::env::temp_dir().join(format!("tuios-beep-{}.wav", uuid::Uuid::new_v4()));
    use std::io::Write;
    match std::fs::File::create(&path) {
        Ok(mut f) => {
            let _ = f.write_all(&data);
            Some(path)
        }
        Err(e) => {
            log::debug!("agent alert: cannot write beep wav: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> AgentAlertsConfig {
        AgentAlertsConfig::default()
    }

    #[test]
    fn defaults_are_quiet() {
        let p = resolve_agent_alerts(&default_config());
        assert!(p.enabled);
        assert!(p.notify);
        assert!(!p.sound);
        assert!(p.dock);
        assert!(p.suppress_focused);
        assert_eq!(p.settle, Duration::from_secs(2));
        assert!(p.alerts("needs_input"));
        assert!(p.alerts("errored"));
        assert!(p.alerts("done"));
        assert!(!p.alerts("idle"));
        assert!(!p.alerts("working"));
        assert!(!p.alerts("bogus"));
        assert!(!p.quiet(0));
    }

    #[test]
    fn explicit_false_survives() {
        let mut c = default_config();
        c.dock = Some(false);
        c.states.done = Some(false);
        let p = resolve_agent_alerts(&c);
        assert!(!p.dock);
        assert!(!p.alerts("done"));
        assert!(p.alerts("needs_input"));
    }

    #[test]
    fn master_switch_silences_everything() {
        let mut c = default_config();
        c.enabled = Some(false);
        let p = resolve_agent_alerts(&c);
        assert!(!p.alerts("needs_input"));
    }

    #[test]
    fn quiet_hours_parse_and_wrap_midnight() {
        let (from, to) = parse_quiet_hours("22:00-08:00").unwrap();
        assert_eq!((from, to), (22 * 60, 8 * 60));
        let mut c = default_config();
        c.quiet_hours = "22:00-08:00".into();
        let p = resolve_agent_alerts(&c);
        assert!(p.quiet(23 * 60 + 30));
        assert!(p.quiet(3 * 60));
        assert!(!p.quiet(12 * 60));
        assert!(parse_quiet_hours("25:00-08:00").is_err());
        assert!(parse_quiet_hours("10:00").is_err());
        assert_eq!(parse_quiet_hours("").unwrap(), (0, 0));
    }

    #[test]
    fn sound_mode_parses() {
        assert_eq!(SoundMode::parse(""), (SoundMode::Audio, true));
        assert_eq!(SoundMode::parse("bell"), (SoundMode::Bell, true));
        assert_eq!(SoundMode::parse("both"), (SoundMode::Both, true));
        assert_eq!(SoundMode::parse("nope"), (SoundMode::Audio, false));
        let mut c = default_config();
        c.sound = Some(true);
        c.sound_mode = "bell".into();
        let p = resolve_agent_alerts(&c);
        assert!(p.plays_bell());
        assert!(!p.plays_audio());
    }

    #[test]
    fn parse_agent_state_validates() {
        assert_eq!(
            parse_agent_state("needs_input").as_deref(),
            Some("needs_input")
        );
        assert_eq!(parse_agent_state("none").as_deref(), Some("none"));
        assert_eq!(parse_agent_state(""), None);
        assert_eq!(parse_agent_state("bogus"), None);
    }

    #[test]
    fn cue_files_resolve() {
        let mut c = default_config();
        c.sounds.done = "/x/done.wav".into();
        c.sounds.needs_input = "/x/attention.wav".into();
        let p = resolve_agent_alerts(&c);
        assert_eq!(p.cue_file("done"), "/x/done.wav");
        assert_eq!(p.cue_file("idle"), "/x/done.wav");
        assert_eq!(p.cue_file("needs_input"), "/x/attention.wav");
        assert_eq!(p.cue_file("errored"), "/x/attention.wav");
    }

    #[test]
    fn sound_cooldown_gates_cues() {
        let mut cue = SoundCue::new();
        // No players in CI are fine; the cooldown logic is what is tested.
        let first = cue.play("", Duration::from_secs(3));
        let second = cue.play("", Duration::from_secs(3));
        assert!(first);
        assert!(!second);
    }
}
