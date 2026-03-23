use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct ElixirProvider;

impl ElixirProvider {
    fn is_phoenix(ctx: &AppContext) -> bool {
        ctx.read_file("mix.exs")
            .ok()
            .is_some_and(|c| c.contains(":phoenix"))
    }
}

impl Provider for ElixirProvider {
    fn name(&self) -> &'static str {
        "elixir"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("mix.exs")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let is_phoenix = Self::is_phoenix(ctx);

        let build_stage = Stage {
            name: "build".to_string(),
            base_image: "elixir:1.18".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                run: "mix local.hex --force && mix local.rebar --force && mix deps.get && MIX_ENV=prod mix release".to_string(),
                cache_mounts: vec![
                    "/root/.mix".to_string(),
                    "/root/.hex".to_string(),
                ],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "debian:bookworm-slim".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![],
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: "/app/_build/prod/rel".to_string(),
                dest: "/app".to_string(),
            }],
            commands: vec![Command {
                run: "apt-get update && apt-get install -y --no-install-recommends libncurses6 libstdc++6 && rm -rf /var/lib/apt/lists/*".to_string(),
                cache_mounts: vec!["/var/cache/apt".to_string()],
            }],
        };

        let start_cmd = if is_phoenix {
            "mix phx.server"
        } else {
            "mix run --no-halt"
        };

        Ok(BuildPlan {
            provider: "elixir".to_string(),
            stages: vec![build_stage, runtime_stage],
            start_command: Some(start_cmd.to_string()),
            port: Some(4000),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_elixir_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mix.exs"), "defmodule MyApp do end").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(ElixirProvider.detect(&ctx));
    }

    #[test]
    fn elixir_plan_two_stages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mix.exs"), "defmodule MyApp do end").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = ElixirProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "elixir");
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(plan.start_command.as_deref(), Some("mix run --no-halt"));
        assert_eq!(plan.port, Some(4000));
    }

    #[test]
    fn detects_phoenix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mix.exs"),
            r#"defmodule MyApp do defp deps do [{:phoenix, "~> 1.7"}] end end"#,
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = ElixirProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("mix phx.server"));
    }
}
