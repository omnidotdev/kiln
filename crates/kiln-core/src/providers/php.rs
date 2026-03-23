use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct PhpProvider;

impl PhpProvider {
    fn is_laravel(ctx: &AppContext) -> bool {
        ctx.has_file("artisan")
            || ctx
                .read_file("composer.json")
                .ok()
                .is_some_and(|c| c.contains("laravel/framework"))
    }
}

impl Provider for PhpProvider {
    fn name(&self) -> &'static str {
        "php"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("composer.json")
    }

    #[allow(clippy::literal_string_with_formatting_args)]
    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let is_laravel = Self::is_laravel(ctx);

        let deps_stage = Stage {
            name: "deps".to_string(),
            base_image: "composer:2".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![
                CopyDirective {
                    src: "composer.json".to_string(),
                    dest: ".".to_string(),
                },
                CopyDirective {
                    src: "composer.lock".to_string(),
                    dest: ".".to_string(),
                },
            ],
            copy_from: vec![],
            commands: vec![Command {
                run: "composer install --no-dev --optimize-autoloader".to_string(),
                cache_mounts: vec!["/root/.composer".to_string()],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "php:8.3-apache".to_string(),
            workdir: "/var/www/html".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![CopyFrom {
                stage: "deps".to_string(),
                src: "/app/vendor".to_string(),
                dest: "/var/www/html/vendor".to_string(),
            }],
            commands: vec![],
        };

        let start_cmd = if is_laravel {
            Some("php artisan serve --host=0.0.0.0 --port=${PORT:-8080}".to_string())
        } else {
            None
        };

        Ok(BuildPlan {
            provider: "php".to_string(),
            stages: vec![deps_stage, runtime_stage],
            start_command: start_cmd,
            port: Some(8080),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_php_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("composer.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(PhpProvider.detect(&ctx));
    }

    #[test]
    fn php_plan_two_stages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("composer.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = PhpProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "php");
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(plan.stages[0].name, "deps");
        assert_eq!(plan.stages[1].name, "runtime");
        assert!(plan.start_command.is_none());
        assert_eq!(plan.port, Some(8080));
    }

    #[test]
    fn detects_laravel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{"require":{"laravel/framework":"^11.0"}}"#,
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = PhpProvider.plan(&ctx).unwrap();
        assert!(plan.start_command.as_ref().unwrap().contains("artisan"));
    }
}
