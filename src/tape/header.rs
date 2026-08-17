//! Project-tape header — the declarative directive block of a `.tuios.tape`,
//! ported from TUIOS `internal/tape/header.go`.

/// Default scope: build the tape in a session named after the project.
pub const SCOPE_SESSION: &str = "session";
/// Apply the tape to the current session, starting from the focused window.
pub const SCOPE_CURRENT: &str = "current";

/// The declarative header of a `.tuios.tape`. Directives may appear only in a
/// leading block, before any action command:
///
/// ```text
/// Session "name"        target session name (default: project directory basename)
/// Scope session|current default session
/// Workspace 2           which workspace inside the session to build in (0 = none)
/// Require "command"     skip-with-notice if a binary is missing (repeatable)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectHeader {
    pub session: String,
    pub scope: String,
    pub workspace: i32,
    pub requires: Vec<String>,
    /// True when at least one recognized directive was parsed, letting callers
    /// tell an explicit header from the defaults applied to a tape with none.
    pub has_header: bool,
}

/// Split a tape's content into its declarative header and the remaining body.
/// The header is the run of leading lines that are blank, a comment, or a
/// recognized directive; the body is everything from the first action command
/// onward, returned verbatim so the lexer parses it unchanged.
///
/// It never executes anything and is robust to hostile input: an unrecognized
/// or malformed directive simply ends the header and starts the body.
pub fn parse_project_header(content: &str) -> (ProjectHeader, String) {
    let mut h = ProjectHeader {
        scope: SCOPE_SESSION.to_string(),
        ..ProjectHeader::default()
    };

    let lines: Vec<&str> = content.split('\n').collect();
    let mut body_start = 0;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            // Blank lines and comments are allowed inside the header block and
            // do not end it (a bare comment tape still runs).
            body_start = i + 1;
            continue;
        }

        let (keyword, rest) = split_first_field(trimmed);
        if !matches!(
            keyword.to_ascii_lowercase().as_str(),
            "session" | "scope" | "workspace" | "require"
        ) {
            // First real command: the header ends here.
            body_start = i;
            break;
        }

        apply_header_directive(&mut h, &keyword, &rest);
        h.has_header = true;
        body_start = i + 1;
    }

    let body = lines[body_start..].join("\n");
    (h, body)
}

/// Fold one directive into the header. Unknown values fall back to a sane
/// default rather than erroring, so a typo never makes a tape unreviewable.
fn apply_header_directive(h: &mut ProjectHeader, keyword: &str, rest: &str) {
    match keyword.to_ascii_lowercase().as_str() {
        "session" => h.session = unquote(rest.trim()),
        "scope" => {
            let v = unquote(rest.trim()).to_ascii_lowercase();
            if v == SCOPE_CURRENT {
                h.scope = SCOPE_CURRENT.to_string();
            } else {
                h.scope = SCOPE_SESSION.to_string();
            }
        }
        "workspace" => {
            if let Ok(n) = unquote(rest.trim()).parse::<i32>() {
                if n >= 0 {
                    h.workspace = n;
                }
            }
        }
        "require" => {
            let cmd = unquote(rest.trim());
            if !cmd.is_empty() {
                h.requires.push(cmd);
            }
        }
        _ => {}
    }
}

/// Split a line into its first whitespace-delimited token and the remainder.
fn split_first_field(s: &str) -> (String, String) {
    match s.find([' ', '\t']) {
        Some(i) => (s[..i].to_string(), s[i + 1..].to_string()),
        None => (s.to_string(), String::new()),
    }
}

/// Strip a single pair of surrounding double or single quotes.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let b = s.as_bytes();
        if (b[0] == b'"' && b[s.len() - 1] == b'"')
            || (b[0] == b'\'' && b[s.len() - 1] == b'\'')
        {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_directives_and_strips_body() {
        let content = "# build session\nSession \"payments\"\nWorkspace 2\nRequire \"cargo\"\nType \"x\"\nEnter";
        let (h, body) = parse_project_header(content);
        assert!(h.has_header);
        assert_eq!(h.session, "payments");
        assert_eq!(h.workspace, 2);
        assert_eq!(h.requires, vec!["cargo"]);
        assert_eq!(h.scope, SCOPE_SESSION);
        assert_eq!(body, "Type \"x\"\nEnter");
    }

    #[test]
    fn scope_current_and_case_insensitive() {
        let (h, _) = parse_project_header("scope CURRENT\nSession 'dev'");
        assert_eq!(h.scope, SCOPE_CURRENT);
        assert_eq!(h.session, "dev");
    }

    #[test]
    fn malformed_directive_ends_header() {
        let (h, body) = parse_project_header("Session \"a\"\nBogusDirective 3\nEnter");
        assert!(h.has_header);
        assert_eq!(h.session, "a");
        assert_eq!(body, "BogusDirective 3\nEnter");
    }

    #[test]
    fn bare_comments_do_not_become_a_header() {
        // A bare comment tape still runs: no directive is ever recognized, so
        // it is not a header, and the comment line is consumed by the scan.
        let (h, body) = parse_project_header("# just a comment\nEnter");
        assert!(!h.has_header);
        assert_eq!(body, "Enter");
    }

    #[test]
    fn no_header_at_all() {
        let (h, body) = parse_project_header("Type \"x\"");
        assert!(!h.has_header);
        assert_eq!(h.scope, SCOPE_SESSION);
        assert_eq!(body, "Type \"x\"");
    }

    #[test]
    fn bad_workspace_falls_back() {
        let (h, _) = parse_project_header("Workspace -3\nEnter");
        assert_eq!(h.workspace, 0);
    }
}
