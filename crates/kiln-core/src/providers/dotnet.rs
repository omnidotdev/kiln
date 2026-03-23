use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct DotnetProvider;

impl DotnetProvider {
    fn find_project_name(ctx: &AppContext) -> String {
        for ext in &["csproj", "fsproj"] {
            for entry in ctx.list_files(".") {
                if entry.extension().is_some_and(|e| e == *ext) {
                    if let Some(stem) = entry.file_stem() {
                        return stem.to_string_lossy().to_string();
                    }
                }
            }
        }

        "app".to_string()
    }
}

impl Provider for DotnetProvider {
    fn name(&self) -> &'static str {
        "dotnet"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file_with_extension("csproj")
            || ctx.has_file_with_extension("fsproj")
            || ctx.has_file_with_extension("sln")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let project_name = Self::find_project_name(ctx);

        let build_stage = Stage {
            name: "build".to_string(),
            base_image: "mcr.microsoft.com/dotnet/sdk:9.0".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                run: "dotnet restore && dotnet publish -c Release -o /out".to_string(),
                cache_mounts: vec!["/root/.nuget".to_string()],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "mcr.microsoft.com/dotnet/aspnet:9.0".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![],
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: "/out".to_string(),
                dest: "/app".to_string(),
            }],
            commands: vec![],
        };

        Ok(BuildPlan {
            provider: "dotnet".to_string(),
            stages: vec![build_stage, runtime_stage],
            start_command: Some(format!("dotnet {project_name}.dll")),
            port: Some(8080),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_csproj() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MyApp.csproj"), "<Project/>").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(DotnetProvider.detect(&ctx));
    }

    #[test]
    fn detects_fsproj() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MyApp.fsproj"), "<Project/>").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(DotnetProvider.detect(&ctx));
    }

    #[test]
    fn dotnet_plan_extracts_project_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("WebApi.csproj"), "<Project/>").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = DotnetProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "dotnet");
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(
            plan.start_command.as_deref(),
            Some("dotnet WebApi.dll")
        );
        assert_eq!(plan.port, Some(8080));
    }
}
