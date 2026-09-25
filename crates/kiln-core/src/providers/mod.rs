mod cpp;
mod deno;
mod dotnet;
mod elixir;
mod gleam;
mod go;
mod java;
mod node;
mod php;
mod python;
mod ruby;
mod rust_lang;
mod shell;
mod static_site;

use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::BuildPlan;

/// Resolve the runtime version for a provider's base image.
///
/// An explicit `version` override wins, then a version auto-detected from the
/// project (`detected`), then the provider's `default`. The result is validated
/// so it is always safe to interpolate into a `FROM` image tag.
///
/// # Errors
///
/// Returns an error if the resolved version contains characters that are not
/// valid in an image tag.
pub(crate) fn resolve_version(ctx: &AppContext, detected: Option<String>, default: &str) -> Result<String> {
    let version = ctx
        .overrides
        .version
        .clone()
        .or(detected)
        .unwrap_or_else(|| default.to_string());
    crate::sanitize::validate_token("version", &version)?;
    Ok(version)
}

/// Read a version from a plain version file (`.nvmrc`, `.python-version`,
/// `.ruby-version`), trimming whitespace and a leading `v`. Returns `None`
/// unless the value looks like a dotted numeric version, so freeform values
/// (e.g. `lts/*`) fall back to the provider default.
pub(crate) fn version_from_file(ctx: &AppContext, file: &str) -> Option<String> {
    ctx.read_file(file).ok().as_deref().and_then(normalize_version)
}

/// Normalize a raw version token: trim whitespace and a leading `v`, and accept
/// it only when it looks like a dotted numeric version (so freeform values such
/// as `lts/*`, `latest`, or `system` fall back to the provider default rather
/// than landing in a `FROM` tag).
pub(crate) fn normalize_version(raw: &str) -> Option<String> {
    let value = raw.trim().trim_start_matches('v').trim();
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.') {
        Some(value.to_string())
    } else {
        None
    }
}

/// Read a tool's version from `.tool-versions` (the asdf/mise format, one
/// `<tool> <version>` per line). `aliases` lists the names the tool may appear
/// under (e.g. `node` and `nodejs`); the first match wins. Comment and blank
/// lines are ignored.
pub(crate) fn version_from_tool_versions(ctx: &AppContext, aliases: &[&str]) -> Option<String> {
    let content = ctx.read_file(".tool-versions").ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(tool) = parts.next() else { continue };
        if aliases.iter().any(|a| a.eq_ignore_ascii_case(tool)) {
            if let Some(version) = parts.next().and_then(normalize_version) {
                return Some(version);
            }
        }
    }
    None
}

/// Read a tool's version from a mise config (`.mise.toml`, `mise.toml`, or
/// `.config/mise/config.toml`) `[tools]` table. The value may be a bare string
/// (`node = "20"`), an inline table (`node = { version = "20" }`), or an array
/// whose first entry is used.
pub(crate) fn version_from_mise(ctx: &AppContext, aliases: &[&str]) -> Option<String> {
    for path in [".mise.toml", "mise.toml", ".config/mise/config.toml"] {
        let Ok(content) = ctx.read_file(path) else { continue };
        let Ok(doc) = content.parse::<toml::Table>() else { continue };
        let Some(tools) = doc.get("tools").and_then(toml::Value::as_table) else {
            continue;
        };
        for alias in aliases {
            let raw = match tools.get(*alias) {
                Some(toml::Value::String(s)) => Some(s.clone()),
                Some(toml::Value::Table(t)) => t.get("version").and_then(toml::Value::as_str).map(str::to_string),
                Some(toml::Value::Array(a)) => a.first().and_then(toml::Value::as_str).map(str::to_string),
                _ => None,
            };
            if let Some(version) = raw.as_deref().and_then(normalize_version) {
                return Some(version);
            }
        }
    }
    None
}

/// Resolve a version from the multi-tool version files shared across the
/// ecosystem (`.tool-versions`, then mise config), under any of `aliases`.
/// Providers consult this after their language-specific single-version file
/// (e.g. `.nvmrc`) and before their built-in default.
pub(crate) fn version_from_tool_files(ctx: &AppContext, aliases: &[&str]) -> Option<String> {
    version_from_tool_versions(ctx, aliases).or_else(|| version_from_mise(ctx, aliases))
}

/// A language/framework provider that can detect and plan builds.
pub trait Provider: Send + Sync {
    /// Provider name (e.g. "node", "go", "python").
    fn name(&self) -> &'static str;

    /// Check if this provider matches the project.
    fn detect(&self, ctx: &AppContext) -> bool;

    /// Generate a build plan for the project.
    ///
    /// # Errors
    ///
    /// Returns an error if plan generation fails.
    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan>;
}


/// Return all registered providers in priority order.
///
/// Order matters: first match wins.
#[must_use]
pub fn all() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(deno::DenoProvider),
        Box::new(gleam::GleamProvider),
        Box::new(elixir::ElixirProvider),
        Box::new(rust_lang::RustProvider),
        Box::new(go::GoProvider),
        Box::new(dotnet::DotnetProvider),
        Box::new(java::JavaProvider),
        Box::new(ruby::RubyProvider),
        Box::new(php::PhpProvider),
        Box::new(python::PythonProvider),
        Box::new(node::NodeProvider),
        Box::new(cpp::CppProvider),
        Box::new(shell::ShellProvider),
        Box::new(static_site::StaticSiteProvider),
    ]
}

#[cfg(test)]
mod version_file_tests {
    use super::*;
    use crate::detect::AppContext;

    fn ctx_with(files: &[(&str, &str)]) -> (tempfile::TempDir, AppContext) {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
        let ctx = AppContext::new(dir.path()).unwrap();
        (dir, ctx)
    }

    #[test]
    fn reads_tool_versions_by_alias() {
        let (_d, ctx) = ctx_with(&[(".tool-versions", "nodejs 20.11.0\npython 3.12.1\n")]);
        assert_eq!(version_from_tool_versions(&ctx, &["node", "nodejs"]).as_deref(), Some("20.11.0"));
        assert_eq!(version_from_tool_versions(&ctx, &["python"]).as_deref(), Some("3.12.1"));
        assert_eq!(version_from_tool_versions(&ctx, &["ruby"]), None);
    }

    #[test]
    fn tool_versions_ignores_comments_and_blank_lines() {
        let (_d, ctx) = ctx_with(&[(".tool-versions", "# a comment\n\n  golang 1.23.4  \n")]);
        assert_eq!(version_from_tool_versions(&ctx, &["go", "golang"]).as_deref(), Some("1.23.4"));
    }

    #[test]
    fn tool_versions_rejects_freeform_values() {
        // `lts` / `latest` are not dotted-numeric, so they fall through to the default
        let (_d, ctx) = ctx_with(&[(".tool-versions", "nodejs lts\n")]);
        assert_eq!(version_from_tool_versions(&ctx, &["node", "nodejs"]), None);
    }

    #[test]
    fn reads_mise_toml_string_table_and_array() {
        let (_d, ctx) = ctx_with(&[(
            "mise.toml",
            "[tools]\nnode = \"20\"\npython = { version = \"3.12\" }\ngo = [\"1.23\", \"1.22\"]\n",
        )]);
        assert_eq!(version_from_mise(&ctx, &["node"]).as_deref(), Some("20"));
        assert_eq!(version_from_mise(&ctx, &["python"]).as_deref(), Some("3.12"));
        assert_eq!(version_from_mise(&ctx, &["go", "golang"]).as_deref(), Some("1.23"));
    }

    #[test]
    fn dotfile_mise_config_is_read() {
        let (_d, ctx) = ctx_with(&[(".mise.toml", "[tools]\nruby = \"3.3.5\"\n")]);
        assert_eq!(version_from_mise(&ctx, &["ruby"]).as_deref(), Some("3.3.5"));
    }

    #[test]
    fn tool_files_prefers_tool_versions_over_mise() {
        let (_d, ctx) = ctx_with(&[(".tool-versions", "nodejs 18.19.0\n"), ("mise.toml", "[tools]\nnode = \"22\"\n")]);
        assert_eq!(version_from_tool_files(&ctx, &["node", "nodejs"]).as_deref(), Some("18.19.0"));
    }
}
