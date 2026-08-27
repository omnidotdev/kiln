use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct RubyProvider;

impl RubyProvider {
    fn is_rails(ctx: &AppContext) -> bool {
        ctx.has_file("config/routes.rb") || ctx.read_file("Gemfile").is_ok_and(|c| c.contains("rails"))
    }
}

impl Provider for RubyProvider {
    fn name(&self) -> &'static str {
        "ruby"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("Gemfile")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let is_rails = Self::is_rails(ctx);

        let deps_stage = Stage {
            name: "deps".to_string(),
            base_image: "ruby:3.3".to_string(),
            workdir: "/app".to_string(),
            // Gemfile.lock is a glob so a project without a committed lockfile
            // still builds; Gemfile guarantees a match.
            copy_files: vec![CopyDirective {
                src: "Gemfile Gemfile.lock*".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            // Do NOT cache-mount /usr/local/bundle (the gem install path): a
            // cache mount is not part of the image layer, so the runtime's
            // `COPY --from=deps /usr/local/bundle` would find no gems. Install
            // into the real layer instead.
            commands: vec![Command {
                run: "bundle install --jobs 4 --retry 3".to_string(),
                cache_mounts: vec![],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "ruby:3.3-slim".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![CopyFrom {
                stage: "deps".to_string(),
                src: "/usr/local/bundle".to_string(),
                dest: "/usr/local/bundle".to_string(),
            }],
            commands: vec![],
        };

        let start_cmd = if is_rails {
            "bundle exec rails server -b 0.0.0.0"
        } else {
            "ruby main.rb"
        };

        Ok(BuildPlan {
            provider: "ruby".to_string(),
            stages: vec![deps_stage, runtime_stage],
            start_command: Some(start_cmd.to_string()),
            port: Some(3000),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ruby_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Gemfile"), "source 'https://rubygems.org'").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(RubyProvider.detect(&ctx));
    }

    #[test]
    fn ruby_plan_two_stages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Gemfile"), "source 'https://rubygems.org'").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RubyProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "ruby");
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(plan.stages[0].name, "deps");
        assert_eq!(plan.stages[1].name, "runtime");
        assert_eq!(plan.start_command.as_deref(), Some("ruby main.rb"));
        assert_eq!(plan.port, Some(3000));
    }

    #[test]
    fn gems_land_in_the_image_layer_not_a_cache_mount() {
        // The runtime does `COPY --from=deps /usr/local/bundle`, so the install
        // must NOT write gems into a cache mount (which is excluded from the
        // layer) or the runtime would boot with no gems.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Gemfile"), "source 'https://rubygems.org'").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RubyProvider.plan(&ctx).unwrap();

        let deps = plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert!(deps.commands[0].cache_mounts.is_empty());
        // Gemfile.lock is copied as an optional glob paired with Gemfile.
        assert_eq!(deps.copy_files.len(), 1);
        assert_eq!(deps.copy_files[0].src, "Gemfile Gemfile.lock*");

        let runtime = plan.stages.iter().find(|s| s.name == "runtime").unwrap();
        assert_eq!(runtime.copy_from[0].src, "/usr/local/bundle");
    }

    #[test]
    fn detects_rails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Gemfile"), "source 'https://rubygems.org'\ngem 'rails'").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = RubyProvider.plan(&ctx).unwrap();
        assert_eq!(
            plan.start_command.as_deref(),
            Some("bundle exec rails server -b 0.0.0.0")
        );
    }
}
