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
    let raw = ctx.read_file(file).ok()?;
    let value = raw.trim().trim_start_matches('v').trim();
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.') {
        Some(value.to_string())
    } else {
        None
    }
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
