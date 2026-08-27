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
}

const FIXTURES: &[Fixture] = &[
    // dependency-free Go: go.sum-optional, CGO_ENABLED=0, distroless + exec CMD
    Fixture {
        dir: "go-nodeps",
        port: 8080,
    },
    // lockfile-less, zero-dependency Node: npm install fallback + node_modules guard
    Fixture {
        dir: "node-nolock",
        port: 3000,
    },
    // plain PHP (no composer.lock): apache serves on port 80
    Fixture {
        dir: "php-plain",
        port: 80,
    },
    // Rust binary: copied out of the /app/target cache mount, exec-form CMD
    Fixture {
        dir: "rust-bin",
        port: 8080,
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

    let result = probe(&cid, fx.port);

    // 4. always clean up the container
    let _ = run(Command::new("docker").args(["rm", "-f", &cid]));
    result
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
    let (_, logs, logs_err) = run(Command::new("docker").args(["logs", "--tail", "20", cid]));
    let logs = format!("{logs}{logs_err}");
    Err(format!(
        "probe {url} never succeeded: {last}\n--- container logs ---\n{logs}"
    ))
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
