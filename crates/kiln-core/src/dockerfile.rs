use crate::plan::BuildPlan;

/// Generate a Dockerfile string from a build plan.
///
/// Uses `BuildKit` syntax extensions (cache mounts, multi-stage).
#[must_use]
pub fn generate(plan: &BuildPlan) -> String {
    let mut lines = vec![String::from("# syntax=docker/dockerfile:1")];

    for stage in &plan.stages {
        lines.push(String::new());
        lines.push(format!("FROM {} AS {}", stage.base_image, stage.name));
        lines.push(format!("WORKDIR {}", stage.workdir));

        for copy in &stage.copy_files {
            lines.push(format!("COPY {} {}", copy.src, copy.dest));
        }

        for copy in &stage.copy_from {
            lines.push(format!("COPY --from={} {} {}", copy.stage, copy.src, copy.dest));
        }

        for cmd in &stage.commands {
            if cmd.cache_mounts.is_empty() {
                lines.push(format!("RUN {}", cmd.run));
            } else {
                let mounts: Vec<String> = cmd
                    .cache_mounts
                    .iter()
                    .map(|m| format!("--mount=type=cache,target={m}"))
                    .collect();
                lines.push(format!("RUN {} {}", mounts.join(" "), cmd.run));
            }
        }
    }

    // Expose port if set
    if let Some(port) = plan.port {
        lines.push(String::new());
        lines.push(format!("EXPOSE {port}"));
    }

    // Start command
    if let Some(ref cmd) = plan.start_command {
        lines.push(format!("CMD [\"/bin/sh\", \"-c\", \"{cmd}\"]"));
    }

    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Command, CopyDirective, CopyFrom, Stage};

    fn minimal_plan() -> BuildPlan {
        BuildPlan {
            provider: "test".to_string(),
            stages: vec![Stage {
                name: "runtime".to_string(),
                base_image: "node:22-slim".to_string(),
                workdir: "/app".to_string(),
                commands: vec![],
                copy_files: vec![CopyDirective {
                    src: ". .".to_string(),
                    dest: ".".to_string(),
                }],
                copy_from: vec![],
            }],
            start_command: Some("node index.js".to_string()),
            port: Some(3000),
        }
    }

    #[test]
    fn test_generates_syntax_directive() {
        let output = generate(&minimal_plan());
        assert!(output.starts_with("# syntax=docker/dockerfile:1"));
    }

    #[test]
    fn test_generates_from_and_workdir() {
        let output = generate(&minimal_plan());
        assert!(output.contains("FROM node:22-slim AS runtime"));
        assert!(output.contains("WORKDIR /app"));
    }

    #[test]
    fn test_generates_expose_and_cmd() {
        let output = generate(&minimal_plan());
        assert!(output.contains("EXPOSE 3000"));
        assert!(output.contains("CMD [\"/bin/sh\", \"-c\", \"node index.js\"]"));
    }

    #[test]
    fn test_generates_cache_mounts() {
        let plan = BuildPlan {
            provider: "test".to_string(),
            stages: vec![Stage {
                name: "deps".to_string(),
                base_image: "node:22".to_string(),
                workdir: "/app".to_string(),
                commands: vec![Command {
                    run: "npm ci".to_string(),
                    cache_mounts: vec!["/root/.npm".to_string()],
                }],
                copy_files: vec![],
                copy_from: vec![],
            }],
            start_command: None,
            port: None,
        };

        let output = generate(&plan);
        assert!(output.contains("RUN --mount=type=cache,target=/root/.npm npm ci"));
    }

    #[test]
    fn test_generates_copy_from_stage() {
        let plan = BuildPlan {
            provider: "test".to_string(),
            stages: vec![Stage {
                name: "runtime".to_string(),
                base_image: "node:22-slim".to_string(),
                workdir: "/app".to_string(),
                commands: vec![],
                copy_files: vec![],
                copy_from: vec![CopyFrom {
                    stage: "build".to_string(),
                    src: "/app/dist".to_string(),
                    dest: "/app/dist".to_string(),
                }],
            }],
            start_command: None,
            port: None,
        };

        let output = generate(&plan);
        assert!(output.contains("COPY --from=build /app/dist /app/dist"));
    }
}
