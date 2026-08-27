use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct GleamProvider;

impl Provider for GleamProvider {
    fn name(&self) -> &'static str {
        "gleam"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("gleam.toml")
    }

    fn plan(&self, _ctx: &AppContext) -> Result<BuildPlan> {
        let build_stage = Stage {
            name: "build".to_string(),
            base_image: "ghcr.io/gleam-lang/gleam:v1.7-erlang".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                run: "gleam export erlang-shipment".to_string(),
                cache_mounts: vec!["/root/.cache/gleam".to_string()],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "erlang:27-slim".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![],
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: "/app/build/erlang-shipment".to_string(),
                dest: "/app".to_string(),
            }],
            commands: vec![],
        };

        // `gleam` (the build tool) is not in erlang:27-slim; the erlang-shipment
        // ships a self-contained entrypoint.sh that boots via `erl` (which is
        // present). It is copied into /app, so launch /app/entrypoint.sh run.
        Ok(BuildPlan {
            provider: "gleam".to_string(),
            stages: vec![build_stage, runtime_stage],
            start_command: Some("/app/entrypoint.sh run".to_string()),
            port: Some(8080),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_gleam_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gleam.toml"), "name = \"myapp\"").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(GleamProvider.detect(&ctx));
    }

    #[test]
    fn gleam_plan_two_stages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gleam.toml"), "name = \"myapp\"").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = GleamProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "gleam");
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(plan.stages[0].name, "build");
        assert_eq!(plan.stages[1].name, "runtime");
        assert_eq!(plan.stages[1].base_image, "erlang:27-slim");
        // erlang:27-slim has no `gleam`; the shipment's entrypoint.sh boots it.
        assert_eq!(plan.start_command.as_deref(), Some("/app/entrypoint.sh run"));
        assert_eq!(plan.port, Some(8080));
    }
}
