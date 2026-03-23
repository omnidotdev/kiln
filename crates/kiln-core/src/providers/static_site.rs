use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, CopyDirective, Stage};
use crate::providers::Provider;

pub struct StaticSiteProvider;

impl Provider for StaticSiteProvider {
    fn name(&self) -> &'static str {
        "static"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("index.html")
    }

    fn plan(&self, _ctx: &AppContext) -> Result<BuildPlan> {
        let stage = Stage {
            name: "runtime".to_string(),
            base_image: "nginx:alpine".to_string(),
            workdir: "/usr/share/nginx/html".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![],
        };

        Ok(BuildPlan {
            provider: "static".to_string(),
            stages: vec![stage],
            start_command: None,
            port: Some(80),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_static_site() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(StaticSiteProvider.detect(&ctx));
    }

    #[test]
    fn static_plan_nginx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = StaticSiteProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "static");
        assert_eq!(plan.stages.len(), 1);
        assert_eq!(plan.stages[0].base_image, "nginx:alpine");
        assert!(plan.start_command.is_none());
        assert_eq!(plan.port, Some(80));
    }
}
