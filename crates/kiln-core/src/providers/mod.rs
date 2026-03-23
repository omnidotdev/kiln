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
