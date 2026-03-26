use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct NodeProvider;

impl NodeProvider {
    fn detect_package_manager(ctx: &AppContext) -> PackageManager {
        if ctx.has_file("bun.lockb") || ctx.has_file("bun.lock") {
            PackageManager::Bun
        } else if ctx.has_file("pnpm-lock.yaml") {
            PackageManager::Pnpm
        } else if ctx.has_file("yarn.lock") {
            PackageManager::Yarn
        } else {
            PackageManager::Npm
        }
    }

    fn detect_start_command(ctx: &AppContext, pm: &PackageManager) -> Option<String> {
        let pkg = ctx.read_file("package.json").ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&pkg).ok()?;
        let scripts = parsed.get("scripts")?;

        if scripts.get("start").is_some() {
            return Some(format!("{} start", pm.run_prefix()));
        }

        let main = parsed.get("main")?.as_str()?;
        Some(format!("node {main}"))
    }

    fn has_build_script(ctx: &AppContext) -> bool {
        ctx.read_file("package.json")
            .ok()
            .and_then(|pkg| serde_json::from_str::<serde_json::Value>(&pkg).ok())
            .and_then(|parsed| parsed.get("scripts")?.get("build").cloned())
            .is_some()
    }
}

impl Provider for NodeProvider {
    fn name(&self) -> &'static str {
        "node"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("package.json")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let pm = Self::detect_package_manager(ctx);
        let has_build = Self::has_build_script(ctx);
        let start_cmd = Self::detect_start_command(ctx, &pm);

        let base_image = "node:22-slim".to_string();
        let build_image = "node:22".to_string();

        let install_cmd = pm.install_command();
        let lock_files = pm.lock_files().join(" ");
        let deps_stage = Stage {
            name: "deps".to_string(),
            base_image: build_image.clone(),
            workdir: "/app".to_string(),
            copy_files: vec![
                CopyDirective {
                    src: "package.json".to_string(),
                    dest: ".".to_string(),
                },
                CopyDirective {
                    src: lock_files,
                    dest: ".".to_string(),
                },
            ],
            copy_from: vec![],
            commands: vec![Command {
                run: install_cmd,
                cache_mounts: pm.cache_dirs(),
            }],
        };

        let mut stages = vec![deps_stage];

        if has_build {
            stages.push(Stage {
                name: "build".to_string(),
                base_image: build_image,
                workdir: "/app".to_string(),
                copy_files: vec![CopyDirective {
                    src: ".".to_string(),
                    dest: ".".to_string(),
                }],
                copy_from: vec![CopyFrom {
                    stage: "deps".to_string(),
                    src: "/app/node_modules".to_string(),
                    dest: "/app/node_modules".to_string(),
                }],
                commands: vec![Command {
                    run: format!("{} run build", pm.run_prefix()),
                    cache_mounts: vec![],
                }],
            });
        }

        let runtime_copy_from = if has_build {
            vec![
                CopyFrom {
                    stage: "deps".to_string(),
                    src: "/app/node_modules".to_string(),
                    dest: "/app/node_modules".to_string(),
                },
                CopyFrom {
                    stage: "build".to_string(),
                    src: "/app".to_string(),
                    dest: "/app".to_string(),
                },
            ]
        } else {
            vec![CopyFrom {
                stage: "deps".to_string(),
                src: "/app/node_modules".to_string(),
                dest: "/app/node_modules".to_string(),
            }]
        };

        let mut runtime_copy_files = vec![];
        if !has_build {
            runtime_copy_files.push(CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            });
        }

        stages.push(Stage {
            name: "runtime".to_string(),
            base_image,
            workdir: "/app".to_string(),
            copy_files: runtime_copy_files,
            copy_from: runtime_copy_from,
            commands: vec![],
        });

        Ok(BuildPlan {
            provider: "node".to_string(),
            stages,
            start_command: start_cmd,
            port: Some(3000),
        })
    }
}

enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
    Bun,
}

impl PackageManager {
    fn install_command(&self) -> String {
        match self {
            Self::Npm => "npm ci".to_string(),
            Self::Yarn => "yarn install --frozen-lockfile".to_string(),
            Self::Pnpm => "pnpm install --frozen-lockfile".to_string(),
            Self::Bun => "bun install".to_string(),
        }
    }

    const fn run_prefix(&self) -> &str {
        match self {
            Self::Npm => "npm",
            Self::Yarn => "yarn",
            Self::Pnpm => "pnpm",
            Self::Bun => "bun",
        }
    }

    fn lock_files(&self) -> Vec<String> {
        match self {
            Self::Npm => vec!["package-lock.json".to_string()],
            Self::Yarn => vec!["yarn.lock".to_string()],
            Self::Pnpm => vec!["pnpm-lock.yaml".to_string()],
            Self::Bun => vec!["bun.lock*".to_string()],
        }
    }

    fn cache_dirs(&self) -> Vec<String> {
        match self {
            Self::Npm => vec!["/root/.npm".to_string()],
            Self::Yarn => vec!["/usr/local/share/.cache/yarn".to_string()],
            Self::Pnpm => vec!["/root/.local/share/pnpm/store".to_string()],
            Self::Bun => vec!["/root/.bun/install/cache".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_node_project(dir: &std::path::Path, pm: &str) {
        let pkg = r#"{"name":"test","scripts":{"start":"node server.js","build":"tsc"}}"#;
        std::fs::write(dir.join("package.json"), pkg).unwrap();
        match pm {
            "npm" => std::fs::write(dir.join("package-lock.json"), "{}").unwrap(),
            "bun" => std::fs::write(dir.join("bun.lock"), "").unwrap(),
            "pnpm" => std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap(),
            "yarn" => std::fs::write(dir.join("yarn.lock"), "").unwrap(),
            _ => {}
        }
    }

    #[test]
    fn detects_node_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(NodeProvider.detect(&ctx));
    }

    #[test]
    fn does_not_detect_non_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.go"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(!NodeProvider.detect(&ctx));
    }

    #[test]
    fn detects_npm() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "npm");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "node");
        assert!(plan.stages[0].commands[0].run.contains("npm ci"));
        assert!(
            plan.stages[0].commands[0]
                .cache_mounts
                .contains(&"/root/.npm".to_string())
        );
    }

    #[test]
    fn detects_bun() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "bun");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert!(plan.stages[0].commands[0].run.contains("bun install"));
    }

    #[test]
    fn generates_build_stage_when_build_script_exists() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "npm");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.stages.len(), 3);
        assert_eq!(plan.stages[1].name, "build");
    }

    #[test]
    fn skips_build_stage_when_no_build_script() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = r#"{"name":"test","scripts":{"start":"node index.js"}}"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(plan.stages[0].name, "deps");
        assert_eq!(plan.stages[1].name, "runtime");
    }

    #[test]
    fn detects_start_command() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "npm");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("npm start"));
    }
}
