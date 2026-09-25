use serde::{Deserialize, Serialize};

/// A complete build plan for a detected project.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BuildPlan {
    /// Provider that generated this plan
    pub provider: String,
    /// Build stages (multi-stage Dockerfile)
    pub stages: Vec<Stage>,
    /// Detected or inferred start command
    pub start_command: Option<String>,
    /// Detected or inferred port
    pub port: Option<u16>,
    /// Environment variables to set in the runtime image (from config).
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// Environment variables set in the build stage (from config).
    #[serde(default)]
    pub build_env: std::collections::BTreeMap<String, String>,
    /// Directories to prepend to `PATH` in the runtime image (from config).
    #[serde(default)]
    pub paths: Vec<String>,
    /// `BuildKit` secret ids mounted on build-stage commands (from config).
    #[serde(default)]
    pub secrets: Vec<String>,
}

impl BuildPlan {
    /// Index of the stage where the application is built, so build-time settings
    /// (build env, pre/post hooks) target the right place. That is the stage
    /// named `build` when a provider has a distinct build step (e.g. Node's
    /// deps/build/runtime split), otherwise the first stage.
    #[must_use]
    pub fn build_stage_index(&self) -> usize {
        self.stages.iter().position(|stage| stage.name == "build").unwrap_or(0)
    }
}

/// A single Dockerfile stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    /// Stage name (e.g. "deps", "build", "runtime")
    pub name: String,
    /// Base image for this stage
    pub base_image: String,
    /// Working directory inside the container
    pub workdir: String,
    /// Commands to run
    pub commands: Vec<Command>,
    /// Files to copy from the build context
    pub copy_files: Vec<CopyDirective>,
    /// Files to copy from another stage
    pub copy_from: Vec<CopyFrom>,
}

/// A RUN command with optional cache mounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    /// Shell command to execute
    pub run: String,
    /// Cache mount paths (`BuildKit` `RUN --mount=type=cache`)
    pub cache_mounts: Vec<String>,
}

/// Copy files from the build context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyDirective {
    /// Source path (relative to build context)
    pub src: String,
    /// Destination path inside the container
    pub dest: String,
}

/// Copy files from a previous build stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyFrom {
    /// Stage name to copy from
    pub stage: String,
    /// Source path in the other stage
    pub src: String,
    /// Destination path in this stage
    pub dest: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(name: &str) -> Stage {
        Stage {
            name: name.to_string(),
            base_image: "img".to_string(),
            workdir: "/app".to_string(),
            commands: vec![],
            copy_files: vec![],
            copy_from: vec![],
        }
    }

    fn plan_with(stage_names: &[&str]) -> BuildPlan {
        BuildPlan {
            stages: stage_names.iter().map(|n| stage(n)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn build_stage_index_prefers_a_named_build_stage() {
        // Node: deps / build / runtime -> the build stage is index 1
        assert_eq!(plan_with(&["deps", "build", "runtime"]).build_stage_index(), 1);
    }

    #[test]
    fn build_stage_index_falls_back_to_first_stage() {
        // Two-stage (go/rust: build/runtime) -> index 0 is the build stage
        assert_eq!(plan_with(&["build", "runtime"]).build_stage_index(), 0);
        // Single-stage (deno/static) -> the only stage
        assert_eq!(plan_with(&["runtime"]).build_stage_index(), 0);
        // Defensive: an empty plan resolves to 0 without panicking
        assert_eq!(plan_with(&[]).build_stage_index(), 0);
    }
}
