//! End-to-end build/run verification for the language providers.
//!
//! For each fixture under `tests/fixtures/<name>/`, this drives the real kiln
//! path: generate a Dockerfile (`kiln plan --emit dockerfile`), `docker build`
//! it, run the image, and HTTP-probe it. It proves that a minimal real app per
//! language actually builds AND serves, catching failures that unit tests over
//! the plan cannot (a missing shell in distroless, a cache-mounted artifact, a
//! port mismatch, a lockfile-less project, etc.).
//!
//! Gated behind `KILN_E2E=1` because it needs Docker + `BuildKit` (buildx) and is
//! slow; a plain `cargo test` skips it. In CI it runs as a dedicated job. Add a
//! language by dropping a fixture dir in `tests/fixtures/` and a row in
//! `FIXTURES` below.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

struct Fixture {
    /// Directory under tests/fixtures/.
    dir: &'static str,
    /// Port the app listens on inside the container.
    port: u16,
    /// When true, HTTP-probe the port. When false (a non-server app), just
    /// verify the container builds and runs without crashing (still running or
    /// exited 0).
    http: bool,
}

const FIXTURES: &[Fixture] = &[
    // dependency-free Go: go.sum-optional, CGO_ENABLED=0, distroless + exec CMD
    Fixture {
        dir: "go-nodeps",
        port: 8080,
        http: true,
    },
    // lockfile-less, zero-dependency Node: npm install fallback + node_modules guard
    Fixture {
        dir: "node-nolock",
        port: 3000,
        http: true,
    },
    // plain PHP (no composer.lock): apache serves on port 80
    Fixture {
        dir: "php-plain",
        port: 80,
        http: true,
    },
    // Rust binary: copied out of the /app/target cache mount, exec-form CMD
    Fixture {
        dir: "rust-bin",
        port: 8080,
        http: true,
    },
    // C++/CMake: cmake provisioned on gcc:14, binary launched via ./app, slim runtime
    Fixture {
        dir: "cpp-cmake",
        port: 8080,
        http: true,
    },
    // Ruby/Sinatra: gems must land in the image layer (not a cache mount)
    Fixture {
        dir: "ruby-sinatra",
        port: 3000,
        http: true,
    },
    // Python/Flask: pip install, site-packages copied into slim, gunicorn start
    Fixture {
        dir: "python-flask",
        port: 8000,
        http: true,
    },
    // Java/Spring Boot Gradle: select the executable jar, not the -plain jar
    Fixture {
        dir: "java-spring",
        port: 8080,
        http: true,
    },
    // Elixir OTP release: launch bin/<app>, matching-GLIBC runtime, libssl
    Fixture {
        dir: "elixir-release",
        port: 4000,
        http: true,
    },
    // Gleam erlang-shipment: must launch via entrypoint.sh (no `gleam` binary
    // in the runtime); verifies the app boots rather than crashing
    Fixture {
        dir: "gleam-app",
        port: 0,
        http: false,
    },
];

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Run a command, returning (success, stdout, stderr). The streams are kept
/// separate: kiln writes the Dockerfile to stdout and logs to stderr, so the
/// two must not be merged when capturing output as data.
fn run(cmd: &mut Command) -> (bool, String, String) {
    let out = cmd.output().expect("spawn");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Build the image, run it, and probe `/` until it returns a response or the
/// deadline passes. Returns Ok(()) on a successful probe.
fn verify(fx: &Fixture) -> Result<(), String> {
    let dir = fixtures_root().join(fx.dir);
    let tag = format!("kiln-e2e-{}", fx.dir);
    let dockerfile = dir.join("Dockerfile.kiln");

    // 1. generate the Dockerfile via the real kiln binary (stdout only; logs
    //    go to stderr and must not land in the Dockerfile)
    let (ok, stdout, stderr) = run(Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["plan", "--path"])
        .arg(&dir)
        .args(["--emit", "dockerfile"]));
    if !ok {
        return Err(format!("kiln plan failed: {stderr}"));
    }
    std::fs::write(&dockerfile, stdout.as_bytes()).map_err(|e| e.to_string())?;

    // 2. build it
    let (ok, out, err) = run(Command::new("docker").args([
        "buildx",
        "build",
        "--load",
        "-t",
        &tag,
        "-f",
        &dockerfile.to_string_lossy(),
        &dir.to_string_lossy(),
    ]));
    let _ = std::fs::remove_file(&dockerfile);
    if !ok {
        return Err(format!("docker build failed:\n{out}{err}"));
    }

    // 3. run it, mapping the container port to a random host port
    let (ok, out, err) =
        run(Command::new("docker").args(["run", "-d", "-P", "-e", &format!("PORT={}", fx.port), &tag]));
    if !ok {
        return Err(format!("docker run failed: {out}{err}"));
    }
    let cid = out.trim().to_string();

    let mut result = if fx.http {
        probe(&cid, fx.port)
    } else {
        ran_without_crashing(&cid)
    };
    // On any failure, attach the container logs (a crashed container has no
    // published port, so the probe never even reaches the HTTP check).
    if let Err(e) = &result {
        let (_, logs, logs_err) = run(Command::new("docker").args(["logs", "--tail", "20", &cid]));
        result = Err(format!("{e}\n--- container logs ---\n{logs}{logs_err}"));
    }

    // 4. always clean up the container
    let _ = run(Command::new("docker").args(["rm", "-f", &cid]));
    result
}

/// For a non-server app: give it a moment to boot, then confirm it did not
/// crash -- it is either still running or has exited cleanly (code 0).
fn ran_without_crashing(cid: &str) -> Result<(), String> {
    std::thread::sleep(Duration::from_secs(4));
    let (ok, out, err) =
        run(Command::new("docker").args(["inspect", "-f", "{{.State.Running}} {{.State.ExitCode}}", cid]));
    if !ok {
        return Err(format!("docker inspect failed: {out}{err}"));
    }
    match out.trim() {
        "true 0" | "false 0" => Ok(()),
        other => Err(format!("container did not run cleanly (running exitcode = {other})")),
    }
}

fn probe(cid: &str, port: u16) -> Result<(), String> {
    // resolve the mapped host port
    let (ok, out, err) = run(Command::new("docker").args(["port", cid, &format!("{port}/tcp")]));
    if !ok {
        return Err(format!("docker port failed: {out}{err}"));
    }
    let host_port = out
        .lines()
        .next()
        .and_then(|l| l.rsplit(':').next())
        .map(str::trim)
        .ok_or_else(|| format!("could not parse mapped port from {out:?}"))?
        .to_string();

    let url = format!("http://127.0.0.1:{host_port}/");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last = String::new();
    while Instant::now() < deadline {
        let (ok, _out, err) = run(Command::new("curl").args(["-fsS", "--max-time", "3", &url]));
        if ok {
            return Ok(());
        }
        last = err;
        std::thread::sleep(Duration::from_millis(1000));
    }
    Err(format!("probe {url} never succeeded: {last}"))
}

#[test]
fn e2e_fixtures_build_and_serve() {
    if std::env::var("KILN_E2E").is_err() {
        eprintln!("skipping E2E (set KILN_E2E=1 to run; needs docker + buildx)");
        return;
    }

    let mut failures = Vec::new();
    for fx in FIXTURES {
        match verify(fx) {
            Ok(()) => eprintln!("E2E ok: {}", fx.dir),
            Err(e) => {
                eprintln!("E2E FAIL: {}\n{e}", fx.dir);
                failures.push(fx.dir);
            }
        }
    }
    assert!(failures.is_empty(), "E2E fixtures failed: {failures:?}");
}
