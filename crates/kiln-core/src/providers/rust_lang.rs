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

        let build_stage = Stage {
            name: "build".to_string(),
            base_image: "rust:1.85".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                run: format!("cargo build --release --bin {binary}"),
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
                src: format!("/app/target/release/{binary}"),
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
