//! Shell-command hooks system, ported from TUIOS `internal/hooks/hooks.go`.
//!
//! Hooks fire asynchronously when specific events occur (window creation,
//! focus changes, workspace switches, etc.) and execute user-defined shell
//! commands with `TERMOS_*` environment variables providing context.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A hook event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    AfterNewWindow,
    AfterCloseWindow,
    AfterFocusChange,
    AfterWorkspaceSwitch,
    AfterAttach,
    AfterDetach,
    AfterLayoutChange,
    AfterResize,
    /// Fires when a pane's agent state changes to one the
    /// `[notifications.agent]` policy alerts on. It is the only event gated by
    /// config rather than by the raw fact, because it is an alert sink: firing
    /// it on every flip would make it the thing people mute.
    AfterAgentState,
}

impl Event {
    /// All valid hook event names.
    pub const ALL: [Event; 9] = [
        Event::AfterNewWindow,
        Event::AfterCloseWindow,
        Event::AfterFocusChange,
        Event::AfterWorkspaceSwitch,
        Event::AfterAttach,
        Event::AfterDetach,
        Event::AfterLayoutChange,
        Event::AfterResize,
        Event::AfterAgentState,
    ];

    /// The wire name, e.g. `after-new-window`.
    pub fn as_str(self) -> &'static str {
        match self {
            Event::AfterNewWindow => "after-new-window",
            Event::AfterCloseWindow => "after-close-window",
            Event::AfterFocusChange => "after-focus-change",
            Event::AfterWorkspaceSwitch => "after-workspace-switch",
            Event::AfterAttach => "after-attach",
            Event::AfterDetach => "after-detach",
            Event::AfterLayoutChange => "after-layout-change",
            Event::AfterResize => "after-resize",
            Event::AfterAgentState => "after-agent-state",
        }
    }

    /// Parse and validate an event name (whitespace-trimmed).
    pub fn parse(name: &str) -> Option<Event> {
        let name = name.trim();
        Self::ALL.iter().copied().find(|e| e.as_str() == name)
    }
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Context provides environment variables passed to hook commands.
///
/// The fields after `window_id` apply to every event. The ones after that are
/// event-specific and stay at their zero value for the events they do not
/// describe, so a hook script can read them unconditionally.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub window_id: String,
    pub window_name: String,
    pub workspace: i32,
    pub session_id: String,
    /// Set by [`Manager::fire`]; `None` in contexts constructed by callers.
    pub event: Option<Event>,
    /// The workspace that was active before an after-workspace-switch.
    /// Zero for every other event.
    pub previous_workspace: i32,
    /// The tiling layout in force after an after-layout-change: one of
    /// `bsp`, `master-stack`, `scrolling` or `floating`. Empty otherwise.
    pub layout: String,
    /// The window's new size in cells after an after-resize.
    /// Zero for every other event.
    pub width: i32,
    pub height: i32,
    /// The state the pane moved into on an after-agent-state, and the one it
    /// came from, both in the wire spelling `set-agent-state` accepts.
    /// `agent_harness` is the harness id the reporting source named and
    /// `agent_message` the free text it carried. All empty for every other
    /// event.
    pub agent_state: String,
    pub prev_agent_state: String,
    pub agent_harness: String,
    pub agent_message: String,
}

impl Context {
    /// The `TERMOS_*` environment pairs for this context.
    pub fn env_pairs(&self) -> Vec<(String, String)> {
        vec![
            (
                "TERMOS_EVENT".to_string(),
                self.event
                    .map(|e| e.as_str().to_string())
                    .unwrap_or_default(),
            ),
            ("TERMOS_WINDOW_ID".to_string(), self.window_id.clone()),
            ("TERMOS_WINDOW_NAME".to_string(), self.window_name.clone()),
            ("TERMOS_WORKSPACE".to_string(), self.workspace.to_string()),
            ("TERMOS_SESSION_ID".to_string(), self.session_id.clone()),
            (
                "TERMOS_PREV_WORKSPACE".to_string(),
                self.previous_workspace.to_string(),
            ),
            ("TERMOS_LAYOUT".to_string(), self.layout.clone()),
            ("TERMOS_WIDTH".to_string(), self.width.to_string()),
            ("TERMOS_HEIGHT".to_string(), self.height.to_string()),
            ("TERMOS_AGENT_STATE".to_string(), self.agent_state.clone()),
            (
                "TERMOS_AGENT_PREV_STATE".to_string(),
                self.prev_agent_state.clone(),
            ),
            (
                "TERMOS_AGENT_HARNESS".to_string(),
                self.agent_harness.clone(),
            ),
            (
                "TERMOS_AGENT_MESSAGE".to_string(),
                self.agent_message.clone(),
            ),
        ]
    }
}

/// A minimal WaitGroup: a counter + condvar (the Rust counterpart of Go's
/// `sync.WaitGroup`).
#[derive(Debug, Default)]
struct WaitGroup {
    count: Mutex<usize>,
    cv: Condvar,
}

impl WaitGroup {
    fn add(&self) {
        *self.count.lock().unwrap() += 1;
    }

    fn done(&self) {
        let mut count = self.count.lock().unwrap();
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.cv.notify_all();
        }
    }

    fn wait(&self) {
        let mut count = self.count.lock().unwrap();
        while *count > 0 {
            count = self.cv.wait(count).unwrap();
        }
    }

    /// Wait up to `timeout`. Returns true when drained.
    fn wait_timeout(&self, timeout: Duration) -> bool {
        let mut count = self.count.lock().unwrap();
        if *count == 0 {
            return true;
        }
        let (guard, _) = self.cv.wait_timeout(count, timeout).unwrap();
        count = guard;
        *count == 0
    }
}

/// Executes one hook command. A field so tests can observe which hooks fired,
/// with what context, without spawning a shell per event.
type Runner = Arc<dyn Fn(&str, &Context) + Send + Sync>;

/// Manages hook registrations and execution.
pub struct Manager {
    hooks: Mutex<HashMap<Event, Vec<String>>>,
    run: Mutex<Runner>,
    /// Tracks running hooks so a caller that needs the side effects to have
    /// landed can join them instead of sleeping.
    in_flight: Arc<WaitGroup>,
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

impl Manager {
    /// Create a new hooks manager.
    pub fn new() -> Self {
        Self {
            hooks: Mutex::new(HashMap::new()),
            run: Mutex::new(Arc::new(execute_hook)),
            in_flight: Arc::new(WaitGroup::default()),
        }
    }

    /// Replace the command runner. Exists for tests: the real runner spawns a
    /// shell, which makes asserting that an event fired with the right payload
    /// both slow and timing-dependent.
    pub fn set_runner<F>(&self, run: F)
    where
        F: Fn(&str, &Context) + Send + Sync + 'static,
    {
        *self.run.lock().unwrap() = Arc::new(run);
    }

    /// Block until every hook fired so far has finished.
    pub fn wait(&self) {
        self.in_flight.wait();
    }

    /// Wait for in-flight hooks, giving up after `timeout`. Exists for the
    /// events fired on the way out: hooks run in their own threads, which the
    /// process exit would otherwise kill before they ran at all. The timeout
    /// is what keeps a hook that never returns from holding the client open.
    pub fn wait_timeout(&self, timeout: Duration) {
        if !self.in_flight.wait_timeout(timeout) {
            log::warn!("hooks: gave up waiting for hooks to finish after {timeout:?}");
        }
    }

    /// Register a shell command to be executed for a given event.
    pub fn register(&self, event: Event, command: impl Into<String>) {
        self.hooks
            .lock()
            .unwrap()
            .entry(event)
            .or_default()
            .push(command.into());
    }

    /// Remove all hooks for a given event.
    pub fn clear(&self, event: Event) {
        self.hooks.lock().unwrap().remove(&event);
    }

    /// Remove all hooks.
    pub fn clear_all(&self) {
        self.hooks.lock().unwrap().clear();
    }

    /// Load hooks from a config map (parsed from the TOML `[hooks]` section).
    /// Keys are event names; values are shell commands (string or array of
    /// strings). Unknown event names are logged and ignored, never fatal.
    pub fn load_from_config(&self, hook_config: &HashMap<String, toml::Value>) {
        let mut hooks = self.hooks.lock().unwrap();
        hooks.clear();

        for (key, val) in hook_config {
            let Some(event) = Event::parse(key) else {
                log::warn!(
                    "hooks: ignoring unknown event \"{key}\" (valid events: {})",
                    Event::ALL
                        .iter()
                        .map(|e| e.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                continue;
            };
            match val {
                toml::Value::String(s) if !s.is_empty() => {
                    hooks.insert(event, vec![s.clone()]);
                }
                toml::Value::Array(items) => {
                    let cmds: Vec<String> = items
                        .iter()
                        .filter_map(|item| match item {
                            toml::Value::String(s) if !s.is_empty() => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    if !cmds.is_empty() {
                        hooks.insert(event, cmds);
                    }
                }
                _ => {}
            }
        }
    }

    /// Execute all hooks registered for the given event asynchronously. Each
    /// hook runs in its own thread with the provided context as env vars, and
    /// `Fire` returns before any of them finish.
    pub fn fire(&self, event: Event, ctx: Context) {
        let commands = self
            .hooks
            .lock()
            .unwrap()
            .get(&event)
            .cloned()
            .unwrap_or_default();
        let run = self.run.lock().unwrap().clone();
        if commands.is_empty() {
            return;
        }

        let mut ctx = ctx;
        ctx.event = Some(event);
        let in_flight = Arc::clone(&self.in_flight);

        for cmd in commands {
            let run = run.clone();
            let ctx = ctx.clone();
            let in_flight = Arc::clone(&in_flight);
            in_flight.add();
            std::thread::spawn(move || {
                run(&cmd, &ctx);
                in_flight.done();
            });
        }
    }

    /// True if any hooks are registered.
    pub fn has_hooks(&self) -> bool {
        !self.hooks.lock().unwrap().is_empty()
    }
}

/// Run a shell command with context as environment variables. Output is
/// discarded and the exit status ignored (fire-and-forget semantics).
fn execute_hook(cmd_str: &str, ctx: &Context) {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(cmd_str);
    for (key, value) in ctx.env_pairs() {
        cmd.env(key, value);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let _ = cmd.status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_fire_runs_the_shell_command() {
        let m = Manager::new();
        let dir = std::env::temp_dir().join(format!("tuios-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("hook_fired");
        let _ = std::fs::remove_file(&marker);

        m.register(Event::AfterNewWindow, format!("touch {}", marker.display()));
        m.fire(
            Event::AfterNewWindow,
            Context {
                window_id: "test-123".into(),
                workspace: 1,
                ..Context::default()
            },
        );
        m.wait();

        assert!(
            marker.exists(),
            "hook did not fire: marker file not created"
        );
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn load_from_config_accepts_string_and_array() {
        let m = Manager::new();
        let mut config = HashMap::new();
        config.insert(
            "after-new-window".to_string(),
            toml::Value::String("echo new".into()),
        );
        config.insert(
            "after-close-window".to_string(),
            toml::Value::Array(vec![
                toml::Value::String("echo close1".into()),
                toml::Value::String("echo close2".into()),
            ]),
        );
        config.insert("not-an-event".to_string(), toml::Value::String("x".into()));

        m.load_from_config(&config);

        assert!(m.has_hooks());
        let hooks = m.hooks.lock().unwrap();
        assert_eq!(hooks[&Event::AfterNewWindow].len(), 1);
        assert_eq!(hooks[&Event::AfterCloseWindow].len(), 2);
        assert!(!hooks.contains_key(&Event::AfterAttach));
    }

    #[test]
    fn clear_removes_hooks_for_an_event() {
        let m = Manager::new();
        m.register(Event::AfterNewWindow, "echo test");
        m.clear(Event::AfterNewWindow);
        assert!(!m.has_hooks());
    }

    #[test]
    fn parse_event_name_validates() {
        assert_eq!(
            Event::parse("after-new-window"),
            Some(Event::AfterNewWindow)
        );
        assert_eq!(Event::parse("  after-attach  "), Some(Event::AfterAttach));
        assert_eq!(Event::parse("invalid-event"), None);
        assert_eq!(Event::parse(""), None);
    }

    #[test]
    fn context_env_vars_reach_the_hook() {
        let m = Manager::new();
        let dir = std::env::temp_dir().join(format!("tuios-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env_file = dir.join("env.txt");
        let _ = std::fs::remove_file(&env_file);

        m.register(
            Event::AfterFocusChange,
            format!("env | grep TERMOS_ > {}", env_file.display()),
        );
        m.fire(
            Event::AfterFocusChange,
            Context {
                window_id: "win-abc".into(),
                window_name: "MyWindow".into(),
                workspace: 3,
                session_id: "sess-xyz".into(),
                ..Context::default()
            },
        );
        m.wait();

        let content = std::fs::read_to_string(&env_file).expect("env file written");
        for want in [
            "TERMOS_EVENT=after-focus-change",
            "TERMOS_WINDOW_ID=win-abc",
            "TERMOS_WINDOW_NAME=MyWindow",
            "TERMOS_WORKSPACE=3",
            "TERMOS_SESSION_ID=sess-xyz",
        ] {
            assert!(
                content.contains(want),
                "expected env var {want:?} in output:\n{content}"
            );
        }
    }

    #[test]
    fn agent_state_hook_environment_contract() {
        let m = Manager::new();
        let dir = std::env::temp_dir().join(format!("tuios-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("env2.txt");
        let _ = std::fs::remove_file(&out);

        m.register(
            Event::AfterAgentState,
            format!("env | grep '^TERMOS_' | sort > {}", out.display()),
        );
        m.fire(
            Event::AfterAgentState,
            Context {
                window_id: "w-1".into(),
                window_name: "build".into(),
                workspace: 3,
                session_id: "main".into(),
                agent_state: "needs_input".into(),
                prev_agent_state: "working".into(),
                agent_harness: "claude".into(),
                agent_message: "awaiting approval".into(),
                ..Context::default()
            },
        );
        m.wait();

        let got = std::fs::read_to_string(&out).expect("hook ran");
        for want in [
            "TERMOS_EVENT=after-agent-state",
            "TERMOS_WINDOW_ID=w-1",
            "TERMOS_WINDOW_NAME=build",
            "TERMOS_WORKSPACE=3",
            "TERMOS_SESSION_ID=main",
            "TERMOS_AGENT_STATE=needs_input",
            "TERMOS_AGENT_PREV_STATE=working",
            "TERMOS_AGENT_HARNESS=claude",
            "TERMOS_AGENT_MESSAGE=awaiting approval",
        ] {
            assert!(got.contains(want), "missing {want:?} from hook env:\n{got}");
        }
    }

    #[test]
    fn agent_message_with_shell_metacharacters_is_environment_safe() {
        // An agent message is free text from a harness; it must travel as an
        // environment variable, never argv.
        let m = Manager::new();
        let dir = std::env::temp_dir().join(format!("tuios-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("msg.txt");
        let _ = std::fs::remove_file(&out);
        let nasty = "\"; rm -rf $HOME; echo '";

        m.register(
            Event::AfterAgentState,
            format!("printf '%s' \"$TERMOS_AGENT_MESSAGE\" > {}", out.display()),
        );
        m.fire(
            Event::AfterAgentState,
            Context {
                agent_message: nasty.to_string(),
                ..Context::default()
            },
        );
        m.wait();

        let got = std::fs::read_to_string(&out).expect("hook ran");
        assert_eq!(got, nasty);
    }

    #[test]
    fn fire_does_not_block_the_caller() {
        let m = Manager::new();
        m.register(Event::AfterAgentState, "sleep 30");
        let started = std::time::Instant::now();
        m.fire(Event::AfterAgentState, Context::default());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "fire blocked on a slow hook"
        );
    }

    #[test]
    fn wait_timeout_gives_up_on_slow_hooks() {
        let m = Manager::new();
        m.register(Event::AfterAgentState, "sleep 30");
        m.fire(Event::AfterAgentState, Context::default());
        let started = std::time::Instant::now();
        m.wait_timeout(std::time::Duration::from_millis(50));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn set_runner_observes_fired_hooks() {
        let m = Manager::new();
        let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        m.set_runner(move |cmd, ctx| {
            seen2
                .lock()
                .unwrap()
                .push((cmd.to_string(), ctx.window_id.clone()));
        });
        m.register(Event::AfterNewWindow, "cmd-1");
        m.register(Event::AfterNewWindow, "cmd-2");
        m.fire(Event::AfterNewWindow, Context::default());
        m.wait();
        // `wait` guarantees the in-flight count reached zero, which happens
        // after the runner returns, so the entries are visible.
        let entries = seen.lock().unwrap();
        assert_eq!(entries.len(), 2);
    }
}
