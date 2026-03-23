use serde::{Deserialize, Serialize};

/// A complete build plan for a detected project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildPlan {
    /// Provider that generated this plan
    pub provider: String,
    /// Build stages (multi-stage Dockerfile)
    pub stages: Vec<Stage>,
    /// Detected or inferred start command
    pub start_command: Option<String>,
    /// Detected or inferred port
    pub port: Option<u16>,
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
