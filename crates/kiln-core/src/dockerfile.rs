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
        lines.push(cmd_line(cmd));
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Render the `CMD` line for a start command.
///
/// Uses exec form (`CMD ["a", "b"]`) when the command needs no shell, so it
/// runs on shell-less runtimes like distroless (the Go/Rust targets), where
/// `/bin/sh` does not exist and a `sh -c` wrapper would crash the container.
/// Falls back to `sh -c` only when the command needs a shell: env-var expansion
/// (`$`), a `VAR=val` prefix (`=`), or operators/globs. Every runtime that
/// receives such a command (python-slim, php-apache, debian-slim) has a shell.
fn cmd_line(cmd: &str) -> String {
    const SHELL_CHARS: &[char] = &[
        '$', '`', '&', '|', ';', '<', '>', '(', ')', '{', '}', '*', '?', '~', '!', '#', '=', '\n',
    ];
    if cmd.contains(SHELL_CHARS) {
        format!("CMD [\"/bin/sh\", \"-c\", \"{cmd}\"]")
    } else {
        let argv = cmd
            .split_whitespace()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("CMD [{argv}]")
    }
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
        // no shell features -> exec form, so it runs on shell-less runtimes
        assert!(output.contains("CMD [\"node\", \"index.js\"]"));
    }

    #[test]
    fn exec_form_cmd_for_shell_less_binary() {
        // a bare binary path (the Go/distroless case) must be exec form, since
        // distroless has no /bin/sh for a `sh -c` wrapper to exec.
        assert_eq!(cmd_line("/bin/app"), "CMD [\"/bin/app\"]");
        assert_eq!(
            cmd_line("bundle exec rails server -b 0.0.0.0"),
            "CMD [\"bundle\", \"exec\", \"rails\", \"server\", \"-b\", \"0.0.0.0\"]"
        );
    }

    #[test]
    #[allow(clippy::literal_string_with_formatting_args)]
    fn shell_form_cmd_when_a_shell_is_needed() {
        // env-var expansion needs a shell
        assert_eq!(
            cmd_line("uvicorn main:app --port ${PORT:-8000}"),
            "CMD [\"/bin/sh\", \"-c\", \"uvicorn main:app --port ${PORT:-8000}\"]"
        );
        // a VAR=val prefix needs a shell too
        assert_eq!(
            cmd_line("PHX_SERVER=true /app/bin/web start"),
            "CMD [\"/bin/sh\", \"-c\", \"PHX_SERVER=true /app/bin/web start\"]"
        );
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
