use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, Stage};
use crate::providers::Provider;

pub struct DenoProvider;

impl DenoProvider {
    fn detect_start_command(ctx: &AppContext) -> Option<String> {
        for config in &["deno.json", "deno.jsonc"] {
            if let Ok(content) = ctx.read_file(config) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    if parsed
                        .get("tasks")
                        .and_then(|t| t.get("start"))
                        .is_some()
                    {
                        return Some("deno task start".to_string());
                    }
                }
            }
        }

        None
    }
}

impl Provider for DenoProvider {
    fn name(&self) -> &'static str {
        "deno"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("deno.json") || ctx.has_file("deno.jsonc")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let start_cmd =
            Self::detect_start_command(ctx)
                .unwrap_or_else(|| "deno run --allow-net main.ts".to_string());

        let stage = Stage {
            name: "runtime".to_string(),
            base_image: "denoland/deno:2".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                run: "deno cache main.ts || true".to_string(),
                cache_mounts: vec!["/root/.cache/deno".to_string()],
            }],
        };

        Ok(BuildPlan {
            provider: "deno".to_string(),
            stages: vec![stage],
            start_command: Some(start_cmd),
            port: Some(8000),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_deno_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("deno.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(DenoProvider.detect(&ctx));
    }

    #[test]
    fn detects_deno_jsonc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("deno.jsonc"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(DenoProvider.detect(&ctx));
    }

    #[test]
    fn deno_plan_single_stage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("deno.json"),
            r#"{"tasks":{"start":"deno run --allow-net server.ts"}}"#,
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = DenoProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "deno");
        assert_eq!(plan.stages.len(), 1);
        assert_eq!(plan.start_command.as_deref(), Some("deno task start"));
        assert_eq!(plan.port, Some(8000));
    }

    #[test]
    fn deno_fallback_start_command() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("deno.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = DenoProvider.plan(&ctx).unwrap();
        assert_eq!(
            plan.start_command.as_deref(),
            Some("deno run --allow-net main.ts")
        );
    }
}
