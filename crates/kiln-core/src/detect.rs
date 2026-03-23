use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::plan::BuildPlan;
use crate::providers;

/// Context for a project being analyzed.
#[derive(Debug)]
pub struct AppContext {
    /// Root path of the project
    pub root: PathBuf,
}

impl AppContext {
    /// Create a new app context rooted at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.exists() {
            return Err(Error::ReadFile {
                path: root,
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "path does not exist"),
            });
        }
        Ok(Self { root })
    }

    /// Check if a file exists relative to the project root.
    #[must_use]
    pub fn has_file(&self, name: &str) -> bool {
        self.root.join(name).is_file()
    }

    /// Read a file relative to the project root.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    pub fn read_file(&self, name: &str) -> Result<String> {
        let path = self.root.join(name);
        std::fs::read_to_string(&path).map_err(|e| Error::ReadFile { path, source: e })
    }

    /// List files in a directory relative to the project root.
    #[must_use]
    pub fn list_files(&self, dir: &str) -> Vec<PathBuf> {
        let path = self.root.join(dir);
        std::fs::read_dir(&path)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(std::result::Result::ok)
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Check if any file with the given extension exists in the root.
    #[must_use]
    pub fn has_file_with_extension(&self, ext: &str) -> bool {
        self.list_files(".")
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == ext))
    }
}

/// Detect the project language and generate a build plan.
///
/// Tries each registered provider in priority order and returns
/// the plan from the first matching provider.
///
/// # Errors
///
/// Returns `NoProviderDetected` if no provider matches.
pub fn detect_and_plan(root: impl AsRef<Path>) -> Result<BuildPlan> {
    let ctx = AppContext::new(root.as_ref())?;

    for provider in providers::all() {
        if provider.detect(&ctx) {
            tracing::info!(provider = provider.name(), "detected project language");
            return provider.plan(&ctx);
        }
    }

    Err(Error::NoProviderDetected(ctx.root))
}

/// Detect the project language without generating a plan.
///
/// Returns the provider name if detected.
///
/// # Errors
///
/// Returns an error if the project root does not exist.
pub fn detect(root: impl AsRef<Path>) -> Result<Option<String>> {
    let ctx = AppContext::new(root.as_ref())?;

    for provider in providers::all() {
        if provider.detect(&ctx) {
            return Ok(Some(provider.name().to_string()));
        }
    }

    Ok(None)
}
