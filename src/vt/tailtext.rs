//! Tail text extraction — ported from Go TUIOS `internal/vt/tailtext.go`.
//!
//! Returns the last N non-blank rows of the active screen.

use crate::vt::screen::ScreenBuffer;

/// Return the last `n` rows of the active screen that carry text, in
/// reading order (top to bottom). Blank rows are skipped.
pub fn tail_text(screen: &ScreenBuffer, n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }

    let height = screen.height();
    let mut collected: Vec<String> = Vec::new();

    for y in (0..height).rev() {
        let text = screen.line_text(y);
        let trimmed = text.trim_end();
        if !trimmed.is_empty() {
            collected.push(trimmed.to_string());
            if collected.len() >= n {
                break;
            }
        }
    }

    collected.reverse();
    collected
}

/// Return the last `n` non-blank rows from a 2D array of cell strings.
/// Useful for testing without constructing a full Screen.
pub fn tail_text_raw(cells: &[Vec<String>], n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }

    let mut collected: Vec<String> = Vec::new();

    for row in cells.iter().rev() {
        let joined: String = row.join("");
        let trimmed = joined.trim_end();
        if !trimmed.is_empty() {
            collected.push(trimmed.to_string());
            if collected.len() >= n {
                break;
            }
        }
    }

    collected.reverse();
    collected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_empty() {
        assert!(tail_text_raw(&[], 5).is_empty());
    }

    #[test]
    fn raw_all_blank() {
        let cells = vec![vec![" ".to_string(); 5], vec![" ".to_string(); 5]];
        assert!(tail_text_raw(&cells, 5).is_empty());
    }

    #[test]
    fn raw_mixed() {
        let cells = vec![
            vec!["h".to_string(), "i".to_string(), " ".to_string()],
            vec![" ".to_string(); 3],
            vec!["b".to_string(), "y".to_string(), "e".to_string()],
        ];
        let result = tail_text_raw(&cells, 5);
        assert_eq!(result, vec!["hi", "bye"]);
    }

    #[test]
    fn raw_n_zero() {
        let cells = vec![vec!["a".to_string()]];
        assert!(tail_text_raw(&cells, 0).is_empty());
    }

    #[test]
    fn raw_n_larger_than_content() {
        let cells = vec![vec!["x".to_string()]];
        let result = tail_text_raw(&cells, 10);
        assert_eq!(result, vec!["x"]);
    }

    #[test]
    fn raw_n_smaller_than_content() {
        let cells = vec![
            vec!["a".to_string()],
            vec!["b".to_string()],
            vec!["c".to_string()],
        ];
        let result = tail_text_raw(&cells, 2);
        assert_eq!(result, vec!["b", "c"]);
    }

    #[test]
    fn raw_trims_trailing_whitespace() {
        let cells = vec![vec![
            "h".to_string(),
            "i".to_string(),
            " ".to_string(),
            " ".to_string(),
        ]];
        let result = tail_text_raw(&cells, 5);
        assert_eq!(result, vec!["hi"]);
    }
}
