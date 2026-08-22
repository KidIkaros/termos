//! Structured notification pipeline.
//!
//! Notifications flow through a pipeline:
//! 1. **Trigger** → generates a notification event
//! 2. **Template** → renders the notification body
//! 3. **Rate limit** → prevents flooding
//! 4. **Dispatch** → sends to one or more channels
//!
//! Based on the notification system design from Chapter 10 of System Design Interview.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::notifications_channel::Channel;

/// A notification template with variable substitution.
#[derive(Debug, Clone)]
pub struct NotificationTemplate {
    /// Template name (e.g., "pty_exit", "session_start").
    pub name: String,
    /// Title template with {variables}.
    pub title: String,
    /// Body template with {variables}.
    pub body: String,
    /// Default priority level.
    pub priority: Priority,
}

/// Notification priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Critical => "critical",
        }
    }
}

/// A rendered notification ready for dispatch.
#[derive(Debug, Clone)]
pub struct RenderedNotification {
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub channel: Channel,
    pub timestamp: Instant,
    pub event_id: u64,
}

/// The notification pipeline manages templates, rate limiting, and dispatch.
pub struct NotificationPipeline {
    templates: HashMap<String, NotificationTemplate>,
    pub(crate) rate_limiter: crate::util::TokenBucket,
    event_counter: std::sync::atomic::AtomicU64,
    history: Arc<Mutex<Vec<RenderedNotification>>>,
}

impl NotificationPipeline {
    /// Create a new pipeline with default templates.
    pub fn new() -> Self {
        let mut templates = HashMap::new();

        templates.insert(
            "pty_exit".to_string(),
            NotificationTemplate {
                name: "pty_exit".to_string(),
                title: "Process exited".to_string(),
                body: "{command} exited with code {code}".to_string(),
                priority: Priority::Normal,
            },
        );

        templates.insert(
            "session_start".to_string(),
            NotificationTemplate {
                name: "session_start".to_string(),
                title: "Session started".to_string(),
                body: "Session '{name}' is now active".to_string(),
                priority: Priority::Low,
            },
        );

        templates.insert(
            "session_error".to_string(),
            NotificationTemplate {
                name: "session_error".to_string(),
                title: "Session error".to_string(),
                body: "{error}".to_string(),
                priority: Priority::High,
            },
        );

        templates.insert(
            "hook_failed".to_string(),
            NotificationTemplate {
                name: "hook_failed".to_string(),
                title: "Hook failed".to_string(),
                body: "Hook '{hook}' failed: {error}".to_string(),
                priority: Priority::High,
            },
        );

        templates.insert(
            "config_reload".to_string(),
            NotificationTemplate {
                name: "config_reload".to_string(),
                title: "Config reloaded".to_string(),
                body: "Configuration changes applied".to_string(),
                priority: Priority::Low,
            },
        );

        templates.insert(
            "daemon_connected".to_string(),
            NotificationTemplate {
                name: "daemon_connected".to_string(),
                title: "Daemon connected".to_string(),
                body: "Connected to session daemon at {addr}".to_string(),
                priority: Priority::Normal,
            },
        );

        Self {
            templates,
            // 10 notifications burst, refill 2/sec
            rate_limiter: crate::util::TokenBucket::new(10.0, 2.0),
            event_counter: std::sync::atomic::AtomicU64::new(0),
            history: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Render a notification from a template.
    pub fn render(
        &self,
        template_name: &str,
        variables: &HashMap<String, String>,
        channel: Channel,
    ) -> Option<RenderedNotification> {
        let template = self.templates.get(template_name)?;

        let event_id = self
            .event_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let title = Self::substitute(&template.title, variables);
        let body = Self::substitute(&template.body, variables);

        Some(RenderedNotification {
            title,
            body,
            priority: template.priority,
            channel,
            timestamp: Instant::now(),
            event_id,
        })
    }

    /// Render and dispatch a notification. Returns `true` if sent.
    pub fn notify(
        &self,
        template_name: &str,
        variables: &HashMap<String, String>,
        channel: Channel,
    ) -> bool {
        if !self.rate_limiter.try_acquire() {
            return false; // Rate limited
        }

        if let Some(notification) = self.render(template_name, variables, channel) {
            let mut history = self.history.lock().unwrap();
            history.push(notification.clone());
            // Keep last 100 notifications
            if history.len() > 100 {
                history.remove(0);
            }
            true
        } else {
            false
        }
    }

    /// Get recent notification history.
    pub fn history(&self) -> Vec<RenderedNotification> {
        self.history.lock().unwrap().clone()
    }

    /// Register a custom template.
    pub fn register_template(&mut self, template: NotificationTemplate) {
        self.templates.insert(template.name.clone(), template);
    }

    /// Check if a template exists.
    pub fn has_template(&self, name: &str) -> bool {
        self.templates.contains_key(name)
    }

    /// Substitute {variables} in a template string.
    fn substitute(template: &str, variables: &HashMap<String, String>) -> String {
        let mut result = template.to_string();
        for (key, value) in variables {
            let placeholder = format!("{{{key}}}");
            result = result.replace(&placeholder, value);
        }
        result
    }
}

impl Default for NotificationPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_variables() {
        let pipeline = NotificationPipeline::new();
        let mut vars = HashMap::new();
        vars.insert("command".to_string(), "ls".to_string());
        vars.insert("code".to_string(), "0".to_string());

        let notification = pipeline
            .render("pty_exit", &vars, Channel::TuiOverlay)
            .unwrap();
        assert_eq!(notification.title, "Process exited");
        assert_eq!(notification.body, "ls exited with code 0");
    }

    #[test]
    fn notify_respects_rate_limit() {
        let mut pipeline = NotificationPipeline::new();
        pipeline.rate_limiter = crate::util::TokenBucket::new(3.0, 0.0); // No refill

        let vars = HashMap::new();
        // Should allow 3 notifications
        for _ in 0..3 {
            assert!(pipeline.notify("config_reload", &vars, Channel::TuiOverlay));
        }
        // 4th should be rate limited
        assert!(!pipeline.notify("config_reload", &vars, Channel::TuiOverlay));
    }

    #[test]
    fn history_is_tracked() {
        let pipeline = NotificationPipeline::new();
        let vars = HashMap::new();
        pipeline.notify("config_reload", &vars, Channel::TuiOverlay);
        pipeline.notify("config_reload", &vars, Channel::TuiOverlay);

        let history = pipeline.history();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
    }

    #[test]
    fn unknown_template_returns_none() {
        let pipeline = NotificationPipeline::new();
        let vars = HashMap::new();
        assert!(pipeline.render("nonexistent", &vars, Channel::TuiOverlay).is_none());
    }
}
