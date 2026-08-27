use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct ElixirProvider;

impl ElixirProvider {
    fn is_phoenix(ctx: &AppContext) -> bool {
        ctx.read_file("mix.exs").is_ok_and(|c| c.contains(":phoenix"))
    }

    /// OTP application name from mix.exs `app: :name` (the release is named after
    /// it). Falls back to "app". Matches `app:` only when the preceding char is
    /// not part of a longer identifier, so `applications:` / `myapp:` do not
    /// match, then reads the `:atom` that follows.
    fn app_name(ctx: &AppContext) -> String {
        let content = ctx.read_file("mix.exs").unwrap_or_default();
        let bytes = content.as_bytes();
        let mut from = 0;
        while let Some(rel) = content[from..].find("app:") {
            let idx = from + rel;
            let standalone_key = idx == 0 || !matches!(bytes[idx - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_');
            if standalone_key {
                if let Some(atom) = content[idx + 4..].trim_start().strip_prefix(':') {
                    let name: String = atom.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() {
                        return name;
                    }
                }
            }
            from = idx + 4;
        }
        "app".to_string()
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
        let app = Self::app_name(ctx);

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
                run: "mix local.hex --force && mix local.rebar --force && mix deps.get && MIX_ENV=prod mix release"
                    .to_string(),
                cache_mounts: vec!["/root/.mix".to_string(), "/root/.hex".to_string()],
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "debian:bookworm-slim".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![],
            // Copy the release itself (`_build/prod/rel/<app>`) into /app so the
            // launcher lands at /app/bin/<app>.
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: format!("/app/_build/prod/rel/{app}"),
                dest: "/app".to_string(),
            }],
            // The OTP release bundles ERTS but its :crypto NIF links
            // libcrypto.so.3, which debian-slim lacks (libssl3), and ncurses/
            // libstdc++ are needed too.
            commands: vec![Command {
                run: "apt-get update && apt-get install -y --no-install-recommends libncurses6 libstdc++6 libssl3 && rm -rf /var/lib/apt/lists/*".to_string(),
                cache_mounts: vec!["/var/cache/apt".to_string()],
            }],
        };

        // Launch the OTP release binary, not `mix` (mix is not in the runtime
        // image). For Phoenix, PHX_SERVER=true makes the release boot the web
        // endpoint (the generated runtime.exs gates the server on it).
        let start_cmd = if is_phoenix {
            format!("PHX_SERVER=true /app/bin/{app} start")
        } else {
            format!("/app/bin/{app} start")
        };

        Ok(BuildPlan {
            provider: "elixir".to_string(),
            stages: vec![build_stage, runtime_stage],
            start_command: Some(start_cmd),
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
    fn elixir_plan_launches_release_binary_not_mix() {
        // mix is not in the runtime image; the release must launch via its own
        // bin/<app> launcher, and the app name comes from mix.exs `app:`.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mix.exs"),
            "defmodule MyApp.MixProject do\n  def project do\n    [app: :my_app, version: \"0.1.0\"]\n  end\nend",
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = ElixirProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "elixir");
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(plan.start_command.as_deref(), Some("/app/bin/my_app start"));
        assert_eq!(plan.port, Some(4000));

        // the release dir for this app is copied into /app, and crypto's libssl
        // is installed in the runtime.
        let runtime = plan.stages.iter().find(|s| s.name == "runtime").unwrap();
        assert_eq!(runtime.copy_from[0].src, "/app/_build/prod/rel/my_app");
        assert!(runtime.commands[0].run.contains("libssl3"));
    }

    #[test]
    fn app_name_falls_back_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mix.exs"), "defmodule MyApp do end").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = ElixirProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("/app/bin/app start"));
    }

    #[test]
    fn phoenix_release_sets_phx_server() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mix.exs"),
            "defmodule MyApp.MixProject do\n  def project, do: [app: :web]\n  defp deps do [{:phoenix, \"~> 1.7\"}] end\nend",
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = ElixirProvider.plan(&ctx).unwrap();
        assert_eq!(
            plan.start_command.as_deref(),
            Some("PHX_SERVER=true /app/bin/web start")
        );
    }
}
