//! Static peer roster — the membership source that tells the mesh daemon
//! which peers to dial.
//!
//! # File format
//!
//! One peer per line. Example:
//!
//! ```text
//! # This is a comment — ignored.
//!
//! a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2  site-a gateway
//! ABCDEFGHIJKLMNOPQRSTUVWXYZ234567ABCDEFGHIJKLMNOPQRSTUVWXYZ234567AB  site-b
//! deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef
//! ```
//!
//! Rules:
//! - Blank lines are ignored.
//! - Lines whose first non-whitespace character is `#` are ignored (comments).
//! - Each entry line begins with a *token* — a non-empty string with no
//!   internal whitespace. The token is stored verbatim; validation (hex/base32
//!   correctness) is left to the daemon.
//! - Everything after the first run of whitespace following the token is
//!   treated as a free-text label (trimmed).
//! - Duplicate tokens are silently deduplicated: **the first occurrence wins**.
//!   If the first occurrence has no label but a later duplicate does, the first
//!   occurrence (no label) is kept — first-wins is simple and predictable.
//! - An all-comments / all-blank file is valid and returns an empty roster;
//!   this is not an error, because the caller may populate the roster
//!   dynamically after construction.

use std::collections::HashSet;
use std::path::Path;

use thiserror::Error;

/// A single peer entry in the roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerEntry {
    /// The raw peer token (64-char hex node id or base32 address blob).
    /// Stored verbatim; the daemon is responsible for parsing it.
    pub token: String,
    /// Optional human-readable label (free text from the roster file).
    pub label: Option<String>,
}

/// A parsed static peer roster.
#[derive(Debug, Clone, Default)]
pub struct PeerRoster {
    peers: Vec<PeerEntry>,
}

/// Errors returned by [`PeerRoster::load`] and [`PeerRoster::load_from_str`].
#[derive(Debug, Error)]
pub enum RosterError {
    /// I/O error reading the roster file.
    #[error("failed to read roster file: {0}")]
    Io(#[from] std::io::Error),

    /// A token on a non-comment, non-blank line is empty or contains
    /// internal whitespace. (The line itself was not blank/comment.)
    #[error("invalid token on line {line}: {token:?}")]
    InvalidToken { line: usize, token: String },
}

impl PeerRoster {
    /// Parse a roster from the string contents of a roster file.
    ///
    /// Returns an empty roster (no error) if `contents` contains only blank
    /// lines and comments.
    pub fn load_from_str(contents: &str) -> Result<PeerRoster, RosterError> {
        let mut peers: Vec<PeerEntry> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for (idx, raw_line) in contents.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = raw_line.trim();

            // Skip blank lines and comment lines.
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Split into token + optional label.
            let mut parts = trimmed.splitn(2, |c: char| c.is_ascii_whitespace());
            let token = parts.next().unwrap_or("").to_string();
            let label = parts
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            // A non-blank, non-comment line must have a non-empty token with
            // no internal whitespace. `splitn` already ensures the token
            // contains no whitespace; the only remaining bad case is empty.
            if token.is_empty() {
                return Err(RosterError::InvalidToken {
                    line: line_no,
                    token,
                });
            }

            // First-occurrence wins: skip duplicate tokens.
            if seen.contains(&token) {
                continue;
            }
            seen.insert(token.clone());
            peers.push(PeerEntry { token, label });
        }

        Ok(PeerRoster { peers })
    }

    /// Read a roster file from `path` and parse it.
    pub fn load(path: impl AsRef<Path>) -> Result<PeerRoster, RosterError> {
        let contents = std::fs::read_to_string(path)?;
        Self::load_from_str(&contents)
    }

    /// Iterate over the parsed peers.
    pub fn peers(&self) -> &[PeerEntry] {
        &self.peers
    }

    /// Returns `true` if the roster contains no peers.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Number of peers in the roster.
    pub fn len(&self) -> usize {
        self.peers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX64: &str = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
    const HEX64_2: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    const BLOB: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567ABCDEFGHIJKLMNOPQRSTUVWXYZ234567AB";

    #[test]
    fn parses_multi_line_with_labels() {
        let input = format!("{HEX64}  site-a gateway\n{BLOB}  site-b\n{HEX64_2}\n");
        let roster = PeerRoster::load_from_str(&input).unwrap();
        assert_eq!(roster.len(), 3);
        assert_eq!(roster.peers()[0].token, HEX64);
        assert_eq!(roster.peers()[0].label.as_deref(), Some("site-a gateway"));
        assert_eq!(roster.peers()[1].token, BLOB);
        assert_eq!(roster.peers()[1].label.as_deref(), Some("site-b"));
        assert_eq!(roster.peers()[2].token, HEX64_2);
        assert!(roster.peers()[2].label.is_none());
    }

    #[test]
    fn blank_lines_and_comments_ignored() {
        let input = format!("\n# This is a comment\n\n   # indented comment\n{HEX64}  label\n\n");
        let roster = PeerRoster::load_from_str(&input).unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster.peers()[0].token, HEX64);
    }

    #[test]
    fn dedup_by_token_first_wins() {
        // First occurrence has no label; later duplicate has one — first wins.
        let input = format!("{HEX64}\n{HEX64}  duplicate-label\n{HEX64_2}  other\n");
        let roster = PeerRoster::load_from_str(&input).unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster.peers()[0].token, HEX64);
        assert!(
            roster.peers()[0].label.is_none(),
            "first-wins: no label kept"
        );
        assert_eq!(roster.peers()[1].token, HEX64_2);
    }

    #[test]
    fn label_optional_token_only_line() {
        let input = format!("{HEX64}\n");
        let roster = PeerRoster::load_from_str(&input).unwrap();
        assert_eq!(roster.len(), 1);
        assert!(roster.peers()[0].label.is_none());
    }

    #[test]
    fn empty_file_returns_empty_roster() {
        let roster = PeerRoster::load_from_str("").unwrap();
        assert!(roster.is_empty());
        assert_eq!(roster.len(), 0);
    }

    #[test]
    fn all_comments_returns_empty_roster() {
        let input = "# comment one\n# comment two\n\n";
        let roster = PeerRoster::load_from_str(input).unwrap();
        assert!(roster.is_empty());
    }

    #[test]
    fn load_from_temp_file() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "# roster").unwrap();
        writeln!(f, "{HEX64}  node-a").unwrap();
        writeln!(f, "{HEX64_2}").unwrap();
        let roster = PeerRoster::load(f.path()).unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster.peers()[0].label.as_deref(), Some("node-a"));
    }

    #[test]
    fn load_nonexistent_file_returns_io_error() {
        let result = PeerRoster::load("/nonexistent/path/roster.conf");
        assert!(matches!(result, Err(RosterError::Io(_))));
    }
}
