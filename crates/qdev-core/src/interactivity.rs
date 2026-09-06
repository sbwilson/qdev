use serde::{Deserialize, Serialize};

/// Interactivity domain mode per AD-12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interactivity {
    #[default]
    Interactive,
    NonInteractive,
}

impl Interactivity {
    pub fn is_interactive(&self) -> bool {
        matches!(self, Interactivity::Interactive)
    }

    pub fn is_non_interactive(&self) -> bool {
        matches!(self, Interactivity::NonInteractive)
    }

    /// Resolve interactivity mode given the CLI flag, optional environment variable value,
    /// and whether stdin is a TTY.
    ///
    /// Per AD-12:
    /// 1. If `--non-interactive` flag is true -> NonInteractive.
    /// 2. If `QDEV_NONINTERACTIVE` is truthy ("1", "true", "yes") -> NonInteractive.
    ///    Invalid environment variable values safely fall back without error.
    /// 3. If stdin is not a TTY -> NonInteractive.
    /// 4. Otherwise -> Interactive.
    pub fn resolve(flag: bool, env_val: Option<&str>, is_stdin_tty: bool) -> Self {
        if flag {
            return Interactivity::NonInteractive;
        }

        if let Some(mode) = env_val.and_then(Self::parse_env_value) {
            return mode;
        }

        if !is_stdin_tty {
            return Interactivity::NonInteractive;
        }

        Interactivity::Interactive
    }

    /// Parses environment variable string.
    /// Returns Some(NonInteractive) for truthy values ("1", "true", "yes", case-insensitive).
    /// Returns None for invalid or falsey values, falling back safely.
    pub fn parse_env_value(val: &str) -> Option<Self> {
        if Self::is_truthy(val) {
            Some(Interactivity::NonInteractive)
        } else {
            None
        }
    }

    fn is_truthy(val: &str) -> bool {
        let trimmed = val.trim();
        trimmed == "1"
            || trimmed.eq_ignore_ascii_case("true")
            || trimmed.eq_ignore_ascii_case("yes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_overrides_all() {
        assert_eq!(
            Interactivity::resolve(true, Some("0"), true),
            Interactivity::NonInteractive
        );
        assert_eq!(
            Interactivity::resolve(true, None, true),
            Interactivity::NonInteractive
        );
    }

    #[test]
    fn test_env_var_truthy() {
        assert_eq!(
            Interactivity::resolve(false, Some("1"), true),
            Interactivity::NonInteractive
        );
        assert_eq!(
            Interactivity::resolve(false, Some("true"), true),
            Interactivity::NonInteractive
        );
        assert_eq!(
            Interactivity::resolve(false, Some("TRUE"), true),
            Interactivity::NonInteractive
        );
        assert_eq!(
            Interactivity::resolve(false, Some("yes"), true),
            Interactivity::NonInteractive
        );
    }

    #[test]
    fn test_env_var_invalid_safe_fallback() {
        // When env var is invalid, it does not trigger NonInteractive; falls back to TTY check
        assert_eq!(
            Interactivity::resolve(false, Some("bogus"), true),
            Interactivity::Interactive
        );
        assert_eq!(
            Interactivity::resolve(false, Some("0"), true),
            Interactivity::Interactive
        );
        assert_eq!(
            Interactivity::resolve(false, Some("bogus"), false),
            Interactivity::NonInteractive
        );
    }

    #[test]
    fn test_tty_check() {
        assert_eq!(
            Interactivity::resolve(false, None, true),
            Interactivity::Interactive
        );
        assert_eq!(
            Interactivity::resolve(false, None, false),
            Interactivity::NonInteractive
        );
    }

    #[test]
    fn test_serialization() {
        assert_eq!(
            serde_json::to_string(&Interactivity::Interactive).unwrap(),
            "\"interactive\""
        );
        assert_eq!(
            serde_json::to_string(&Interactivity::NonInteractive).unwrap(),
            "\"non_interactive\""
        );
    }
}
