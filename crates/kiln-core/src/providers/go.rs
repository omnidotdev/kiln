use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct GoProvider;

impl GoProvider {
    fn detect_module(ctx: &AppContext) -> Option<String> {
        let content = ctx.read_file("go.mod").ok()?;
        content
            .lines()
            .find(|l| l.starts_with("module "))
            .map(|l| l.trim_start_matches("module ").trim().to_string())
    }

    fn binary_name(ctx: &AppContext) -> String {
        Self::detect_module(ctx)
            .and_then(|m| m.rsplit('/').next().map(String::from))
            .unwrap_or_else(|| "app".to_string())
    }
}

impl Provider for GoProvider {
    fn name(&self) -> &'static str {
        "go"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("go.mod")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let binary = Self::binary_name(ctx);

        let deps_stage = Stage {
            name: "deps".to_string(),
            base_image: "golang:1.24".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![
                CopyDirective {
                    src: "go.mod".to_string(),
                    dest: ".".to_string(),
                },
                CopyDirective {
                    src: "go.sum".to_string(),
                    dest: ".".to_string(),
                },
            ],
            copy_from: vec![],
            commands: vec![Command {
                run: "go mod download".to_string(),
                cache_mounts: vec!["/go/pkg/mod".to_string()],
            }],
        };

        let build_stage = Stage {
            name: "build".to_string(),
            base_image: "golang:1.24".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![CopyFrom {
                stage: "deps".to_string(),
                src: "/go/pkg/mod".to_string(),
                dest: "/go/pkg/mod".to_string(),
            }],
            commands: vec![Command {
                run: format!("CGO_ENABLED=0 go build -o /bin/{binary} ."),
                cache_mounts: vec!["/root/.cache/go-build".to_string()],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "gcr.io/distroless/static-debian12".to_string(),
            workdir: "/".to_string(),
            copy_files: vec![],
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: format!("/bin/{binary}"),
                dest: format!("/bin/{binary}"),
            }],
            commands: vec![],
        };

        Ok(BuildPlan {
            provider: "go".to_string(),
            stages: vec![deps_stage, build_stage, runtime_stage],
            start_command: Some(format!("/bin/{binary}")),
            port: Some(8080),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_go_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module github.com/user/myapp\n\ngo 1.24\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(GoProvider.detect(&ctx));
    }

    #[test]
    fn go_plan_stages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module github.com/user/myapp\n\ngo 1.24\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = GoProvider.plan(&ctx).unwrap();
        assert_eq!(plan.stages.len(), 3);
        assert_eq!(plan.stages[0].name, "deps");
        assert_eq!(plan.stages[1].name, "build");
        assert_eq!(plan.stages[2].name, "runtime");
    }

    #[test]
    fn go_binary_name_from_module() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module github.com/user/myapp\n\ngo 1.24\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = GoProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("/bin/myapp"));
    }

    #[test]
    fn go_uses_distroless_runtime() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = GoProvider.plan(&ctx).unwrap();
        assert!(plan.stages[2].base_image.contains("distroless"));
    }
}
