use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Kiln — container image builder with automatic language detection
#[derive(Parser)]
#[command(name = "kiln", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

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
    },
}

fn main() {
    tracing_subscriber::fmt()
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
        ),
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
    let plan = kiln_core::detect_and_plan(path)?;

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

#[allow(clippy::too_many_lines, clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
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
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Clone source repo if provided
    let work_dir = if let Some(url) = source {
        let tmp = std::env::temp_dir().join("kiln-build");
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp)?;
        }

        let mut cmd = std::process::Command::new("git");
        cmd.args(["clone", "--depth", "1"]);
        if let Some(r) = git_ref {
            cmd.args(["--branch", r]);
        }
        cmd.args([url, &tmp.display().to_string()]);

        let status = cmd.status()?;
        if !status.success() {
            // If branch clone failed, try fetching specific ref (commit SHA)
            if let Some(r) = git_ref {
                let status = std::process::Command::new("git")
                    .args(["clone", url, &tmp.display().to_string()])
                    .status()?;
                if !status.success() {
                    return Err("git clone failed".into());
                }
                let status = std::process::Command::new("git")
                    .args(["checkout", r])
                    .current_dir(&tmp)
                    .status()?;
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

    // Generate or use provided Dockerfile
    let dockerfile_content = if let Some(df) = dockerfile {
        std::fs::read_to_string(df)?
    } else {
        let plan = kiln_core::detect_and_plan(&work_dir)?;
        tracing::info!(provider = plan.provider, "detected language, generating Dockerfile");
        kiln_core::dockerfile::generate(&plan)
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

    args.push("--output".to_string());
    args.push(format!("type=image,name={dest},push=true{insecure_suffix}"));

    args
}

#[cfg(test)]
mod tests {
    use super::build_buildctl_args;
    use std::path::Path;

    #[test]
    fn omits_cache_flags_when_unset() {
        let args = build_buildctl_args(
            "tcp://127.0.0.1:1234",
            Path::new("/workspace"),
            "registry.example/app:abc123",
            None,
            None,
            false,
        );
        assert!(args.iter().all(|a| a != "--import-cache"));
        assert!(args.iter().all(|a| a != "--export-cache"));
        assert!(args
            .iter()
            .any(|a| a == "type=image,name=registry.example/app:abc123,push=true"));
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

        let output_idx = args
            .iter()
            .position(|a| a == "--output")
            .expect("--output present");
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
        );

        let import_idx = args.iter().position(|a| a == "--import-cache").unwrap();
        assert_eq!(
            args[import_idx + 1],
            "type=registry,ref=ghcr.io/owner/app:buildcache",
        );

        let output_idx = args.iter().position(|a| a == "--output").unwrap();
        assert_eq!(
            args[output_idx + 1],
            "type=image,name=ghcr.io/owner/app:abc,push=true",
        );
    }
}
