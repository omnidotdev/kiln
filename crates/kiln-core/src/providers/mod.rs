mod go;
mod node;
mod python;
mod rust_lang;

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
        Box::new(rust_lang::RustProvider),
        Box::new(go::GoProvider),
        Box::new(python::PythonProvider),
        Box::new(node::NodeProvider),
    ]
}
