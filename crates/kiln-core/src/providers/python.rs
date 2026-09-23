use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct PythonProvider;

impl PythonProvider {
    fn detect_package_manager(ctx: &AppContext) -> PythonPm {
        if ctx.has_file("uv.lock") {
            PythonPm::Uv
        } else if ctx.has_file("Pipfile") || ctx.has_file("Pipfile.lock") {
            PythonPm::Pipenv
        } else if ctx.has_file("poetry.lock") || Self::has_poetry_section(ctx) {
            PythonPm::Poetry
        } else if ctx.has_file("pdm.lock") {
            PythonPm::Pdm
        } else {
            PythonPm::Pip
        }
    }

    fn has_poetry_section(ctx: &AppContext) -> bool {
        ctx.read_file("pyproject.toml")
            .is_ok_and(|c| c.contains("[tool.poetry]"))
    }

    fn detect_framework(ctx: &AppContext) -> Option<PythonFramework> {
        let reqs = ctx.read_file("requirements.txt").ok().unwrap_or_default();
        let pyproject = ctx.read_file("pyproject.toml").ok().unwrap_or_default();
        let combined = format!("{reqs}\n{pyproject}");

        if combined.contains("django") || combined.contains("Django") {
            Some(PythonFramework::Django)
        } else if combined.contains("fastapi") || combined.contains("FastAPI") {
            Some(PythonFramework::FastApi)
        } else if combined.contains("flask") || combined.contains("Flask") {
            Some(PythonFramework::Flask)
        } else {
            None
        }
    }

    fn detect_entry_file(ctx: &AppContext) -> Option<String> {
        for name in &["main.py", "app.py", "server.py", "run.py"] {
            if ctx.has_file(name) {
                return Some((*name).to_string());
            }
        }
        None
    }

    // The framework start commands embed the shell `${PORT:-8000}` default
    // expansion, which clippy misreads as a stray format placeholder inside the
    // format! strings; it is a literal shell fragment, not a Rust arg.
    #[allow(clippy::literal_string_with_formatting_args)]
    fn start_command(framework: Option<&PythonFramework>, entry: Option<&str>) -> Option<String> {
        match framework {
            Some(PythonFramework::Django) => {
                Some("gunicorn --bind 0.0.0.0:${PORT:-8000} config.wsgi:application".to_string())
            }
            Some(PythonFramework::FastApi) => {
                let module = entry.unwrap_or("main.py").strip_suffix(".py").unwrap_or("main");
                Some(format!("uvicorn {module}:app --host 0.0.0.0 --port ${{PORT:-8000}}"))
            }
            Some(PythonFramework::Flask) => {
                let module = entry.unwrap_or("app.py").strip_suffix(".py").unwrap_or("app");
                Some(format!("gunicorn --bind 0.0.0.0:${{PORT:-8000}} {module}:app"))
            }
            None => entry.map(|e| format!("python {e}")),
        }
    }
}

impl Provider for PythonProvider {
    fn name(&self) -> &'static str {
        "python"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("requirements.txt")
            || ctx.has_file("pyproject.toml")
            || ctx.has_file("Pipfile")
            || ctx.has_file("setup.py")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let pm = Self::detect_package_manager(ctx);
        let framework = Self::detect_framework(ctx);
        let entry = Self::detect_entry_file(ctx);

        let base_image = "python:3.13-slim".to_string();
        let build_image = "python:3.13".to_string();

        let (copy_files, install_cmd, cache_dirs) = pm.install_info();
        let deps_stage = Stage {
            name: "deps".to_string(),
            base_image: build_image,
            workdir: "/app".to_string(),
            copy_files,
            copy_from: vec![],
            commands: vec![Command {
                run: install_cmd,
                cache_mounts: cache_dirs,
            }],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image,
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![
                CopyFrom {
                    stage: "deps".to_string(),
                    src: SITE_PACKAGES.to_string(),
                    dest: SITE_PACKAGES.to_string(),
                },
                // Console-script entrypoints (gunicorn, uvicorn, ...) install to
                // /usr/local/bin, not site-packages; without this the framework
                // start commands fail with "<tool>: not found".
                CopyFrom {
                    stage: "deps".to_string(),
                    src: "/usr/local/bin".to_string(),
                    dest: "/usr/local/bin".to_string(),
                },
            ],
            commands: vec![],
        };

        let start_cmd = Self::start_command(framework.as_ref(), entry.as_deref());

        Ok(BuildPlan {
            provider: "python".to_string(),
            stages: vec![deps_stage, runtime_stage],
            start_command: start_cmd,
            port: Some(8000),
        })
    }
}

enum PythonPm {
    Pip,
    Poetry,
    Pipenv,
    Uv,
    Pdm,
}

impl PythonPm {
    fn install_info(&self) -> (Vec<CopyDirective>, String, Vec<String>) {
        match self {
            Self::Pip => (
                vec![CopyDirective {
                    src: "requirements.txt".to_string(),
                    dest: ".".to_string(),
                }],
                "pip install --no-cache-dir -r requirements.txt".to_string(),
                vec!["/root/.cache/pip".to_string()],
            ),
            // uv sync installs into /app/.venv, which the runtime stage neither
            // copies onto PATH nor activates. Resolve the locked deps to a
            // requirements file and install them into the system environment
            // instead, so the uniform site-packages + /usr/local/bin copy works.
            Self::Uv => (
                vec![
                    CopyDirective {
                        src: "pyproject.toml".to_string(),
                        dest: ".".to_string(),
                    },
                    CopyDirective {
                        src: "uv.lock".to_string(),
                        dest: ".".to_string(),
                    },
                ],
                "pip install uv && uv export --frozen --no-dev --no-emit-project -o /tmp/requirements.txt && pip install --no-cache-dir -r /tmp/requirements.txt".to_string(),
                vec!["/root/.cache/uv".to_string(), "/root/.cache/pip".to_string()],
            ),
            // POETRY_VIRTUALENVS_CREATE=false forces poetry to install into the
            // system site-packages rather than its own cached virtualenv;
            // --no-root skips installing the project itself (its source is copied
            // into the runtime stage, not built as a package here).
            Self::Poetry => (
                vec![
                    CopyDirective {
                        src: "pyproject.toml".to_string(),
                        dest: ".".to_string(),
                    },
                    CopyDirective {
                        src: "poetry.lock".to_string(),
                        dest: ".".to_string(),
                    },
                ],
                "pip install poetry && POETRY_VIRTUALENVS_CREATE=false poetry install --no-interaction --no-ansi --no-root".to_string(),
                vec!["/root/.cache/pypoetry".to_string()],
            ),
            // pipenv --system installs into the system site-packages instead of
            // a project virtualenv.
            Self::Pipenv => (
                vec![
                    CopyDirective {
                        src: "Pipfile".to_string(),
                        dest: ".".to_string(),
                    },
                    CopyDirective {
                        src: "Pipfile.lock".to_string(),
                        dest: ".".to_string(),
                    },
                ],
                "pip install pipenv && pipenv install --deploy --system".to_string(),
                vec!["/root/.cache/pipenv".to_string()],
            ),
            // pdm's own install uses PEP 582 __pypackages__ or a venv, neither on
            // the system path. Export the locked deps and install them with pip.
            Self::Pdm => (
                vec![
                    CopyDirective {
                        src: "pyproject.toml".to_string(),
                        dest: ".".to_string(),
                    },
                    CopyDirective {
                        src: "pdm.lock".to_string(),
                        dest: ".".to_string(),
                    },
                ],
                "pip install pdm && pdm export --no-hashes -o /tmp/requirements.txt && pip install --no-cache-dir -r /tmp/requirements.txt".to_string(),
                vec!["/root/.cache/pdm".to_string(), "/root/.cache/pip".to_string()],
            ),
        }
    }
}

/// Where installed dependencies land in the deps stage. Every manager is
/// configured to install into the interpreter's system site-packages, so the
/// runtime stage copies one uniform path.
const SITE_PACKAGES: &str = "/usr/local/lib/python3.13/site-packages";

enum PythonFramework {
    Django,
    FastApi,
    Flask,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_requirements_txt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "flask==3.0").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(PythonProvider.detect(&ctx));
    }

    #[test]
    fn detects_pyproject_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(PythonProvider.detect(&ctx));
    }

    #[test]
    fn pip_plan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "flask==3.0").unwrap();
        std::fs::write(dir.path().join("app.py"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = PythonProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "python");
        assert_eq!(plan.stages.len(), 2);
        assert!(plan.stages[0].commands[0].run.contains("pip install"));
    }

    #[test]
    fn detects_flask_start_command() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "flask==3.0").unwrap();
        std::fs::write(dir.path().join("app.py"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = PythonProvider.plan(&ctx).unwrap();
        assert!(plan.start_command.as_ref().unwrap().contains("gunicorn"));
        assert!(plan.start_command.as_ref().unwrap().contains("app:app"));
    }

    #[test]
    fn detects_fastapi_start_command() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "fastapi==0.115").unwrap();
        std::fs::write(dir.path().join("main.py"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = PythonProvider.plan(&ctx).unwrap();
        assert!(plan.start_command.as_ref().unwrap().contains("uvicorn"));
        assert!(plan.start_command.as_ref().unwrap().contains("main:app"));
    }

    #[test]
    fn detects_uv() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]").unwrap();
        std::fs::write(dir.path().join("uv.lock"), "").unwrap();
        std::fs::write(dir.path().join("main.py"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = PythonProvider.plan(&ctx).unwrap();
        assert!(plan.stages[0].commands[0].run.contains("uv export"));
    }

    // Every non-pip manager must resolve dependencies into the SYSTEM
    // site-packages (and /usr/local/bin) that the runtime stage copies. If it
    // installs into its own virtualenv instead, the runtime image has the
    // manager but not the project's dependencies and the app fails to import.
    fn deps_run(dir: &std::path::Path) -> (String, String) {
        let ctx = AppContext::new(dir).unwrap();
        let plan = PythonProvider.plan(&ctx).unwrap();
        (
            plan.stages[0].commands[0].run.clone(),
            plan.stages[1].copy_from[0].src.clone(),
        )
    }

    #[test]
    fn poetry_installs_into_system_site_packages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.poetry]\n").unwrap();
        std::fs::write(dir.path().join("poetry.lock"), "").unwrap();
        let (run, target) = deps_run(dir.path());
        assert!(run.contains("POETRY_VIRTUALENVS_CREATE=false"), "got: {run}");
        assert_eq!(target, SITE_PACKAGES);
    }

    #[test]
    fn uv_installs_into_system_not_venv() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]\n").unwrap();
        std::fs::write(dir.path().join("uv.lock"), "").unwrap();
        let (run, target) = deps_run(dir.path());
        assert!(run.contains("uv export"), "got: {run}");
        assert_eq!(target, SITE_PACKAGES);
    }

    #[test]
    fn pipenv_installs_into_system() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Pipfile"), "").unwrap();
        let (run, target) = deps_run(dir.path());
        assert!(run.contains("--system"), "got: {run}");
        assert_eq!(target, SITE_PACKAGES);
    }

    #[test]
    fn pdm_installs_into_system_site_packages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.pdm]\n").unwrap();
        std::fs::write(dir.path().join("pdm.lock"), "").unwrap();
        let (run, target) = deps_run(dir.path());
        assert!(run.contains("pdm export"), "got: {run}");
        assert_eq!(target, SITE_PACKAGES);
    }
}
