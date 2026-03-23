use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, Stage};
use crate::providers::Provider;

pub struct ShellProvider;

impl ShellProvider {
    fn detect_entry(ctx: &AppContext) -> Option<&'static str> {
        ["main.sh", "start.sh"].iter().copied().find(|name| ctx.has_file(name))
    }
}

impl Provider for ShellProvider {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("main.sh") || ctx.has_file("start.sh")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let entry = Self::detect_entry(ctx).unwrap_or("main.sh");

        let stage = Stage {
            name: "runtime".to_string(),
            base_image: "debian:bookworm-slim".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                run: format!("chmod +x {entry}"),
                cache_mounts: vec![],
            }],
        };

        Ok(BuildPlan {
            provider: "shell".to_string(),
            stages: vec![stage],
            start_command: Some(format!("bash {entry}")),
            port: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_main_sh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.sh"), "#!/bin/bash").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(ShellProvider.detect(&ctx));
    }

    #[test]
    fn detects_start_sh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("start.sh"), "#!/bin/bash").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(ShellProvider.detect(&ctx));
    }

    #[test]
    fn shell_plan_single_stage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.sh"), "#!/bin/bash").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = ShellProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "shell");
        assert_eq!(plan.stages.len(), 1);
        assert_eq!(plan.start_command.as_deref(), Some("bash main.sh"));
        assert!(plan.port.is_none());
    }
}
