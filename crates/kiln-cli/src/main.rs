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
        } => cmd_build(
            source.as_deref(),
            git_ref.as_deref(),
            &dest,
            &path,
            dockerfile.as_deref(),
            &buildkit_addr,
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

#[allow(clippy::too_many_lines)]
fn cmd_build(
    source: Option<&str>,
    git_ref: Option<&str>,
    dest: &str,
    path: &std::path::Path,
    dockerfile: Option<&std::path::Path>,
    buildkit_addr: &str,
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
    let status = std::process::Command::new("buildctl")
        .args([
            "--addr",
            buildkit_addr,
            "build",
            "--frontend",
            "dockerfile.v0",
            "--local",
            &format!("context={}", work_dir.display()),
            "--local",
            &format!("dockerfile={}", work_dir.display()),
            "--opt",
            "filename=Dockerfile.kiln",
            "--output",
            &format!("type=image,name={dest},push=true"),
        ])
        .status()?;

    if !status.success() {
        return Err("buildctl build failed".into());
    }

    tracing::info!(dest, "image built and pushed");
    Ok(())
}
