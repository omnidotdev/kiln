use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct CppProvider;

enum BuildSystem {
    CMake,
    Make,
}

impl CppProvider {
    /// Best-effort executable name for a `CMake` project, read from the first
    /// `add_executable(<name> ...)` target. Falls back to `app` when the target
    /// cannot be parsed or is not a plain identifier (e.g. a `${VAR}` name), so
    /// the value is always safe to interpolate into the Dockerfile.
    fn cmake_executable_name(ctx: &AppContext) -> String {
        let content = ctx.read_file("CMakeLists.txt").unwrap_or_default();
        let lower = content.to_ascii_lowercase();
        let Some(call) = lower.find("add_executable(") else {
            return "app".to_string();
        };
        let after = &content[call + "add_executable(".len()..];
        let name: String = after
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| !c.is_whitespace() && *c != ')')
            .collect();
        if crate::sanitize::validate_token("CMake target", &name).is_ok() {
            name
        } else {
            "app".to_string()
        }
    }
}

impl Provider for CppProvider {
    fn name(&self) -> &'static str {
        "cpp"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("CMakeLists.txt") || ctx.has_file("Makefile")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let build_system = if ctx.has_file("CMakeLists.txt") {
            BuildSystem::CMake
        } else {
            BuildSystem::Make
        };

        // The runtime copies the binary to /app/<name> and launches ./<name>.
        // For CMake the target name comes from add_executable; Make cannot be
        // parsed reliably, so it keeps the `app` convention.
        let binary = match build_system {
            BuildSystem::CMake => Self::cmake_executable_name(ctx),
            BuildSystem::Make => "app".to_string(),
        };

        let (build_cmd, start_cmd) = match build_system {
            BuildSystem::CMake => (
                // gcc:14 (buildpack-deps/Debian) ships gcc and make but NOT
                // cmake, so provision it before invoking the build
                "apt-get update && apt-get install -y --no-install-recommends cmake && cmake -B build && cmake --build build"
                    .to_string(),
                format!("./{binary}"),
            ),
            BuildSystem::Make => ("make".to_string(), format!("./{binary}")),
        };

        let build_stage = Stage {
            name: "build".to_string(),
            base_image: "gcc:14".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![Command {
                run: build_cmd,
                cache_mounts: vec![],
            }],
        };

        // CMake writes the executable into the build tree (build/<name>); a
        // plain Makefile writes it into the source root.
        let copy_src = match build_system {
            BuildSystem::CMake => format!("/app/build/{binary}"),
            BuildSystem::Make => format!("/app/{binary}"),
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            // Match the gcc:14 toolchain's Debian (trixie): a binary built there
            // needs GLIBCXX_3.4.32, which bookworm's older libstdc++ lacks.
            // trixie-slim ships no libstdc++ at all, so install the matching one.
            base_image: "debian:trixie-slim".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![],
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: copy_src,
                dest: format!("/app/{binary}"),
            }],
            commands: vec![Command {
                run: "apt-get update && apt-get install -y --no-install-recommends libstdc++6 ca-certificates && rm -rf /var/lib/apt/lists/*".to_string(),
                cache_mounts: vec!["/var/cache/apt".to_string()],
            }],
        };

        Ok(BuildPlan {
            provider: "cpp".to_string(),
            stages: vec![build_stage, runtime_stage],
            start_command: Some(start_cmd),
            port: Some(8080),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cmake_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CMakeLists.txt"), "project(myapp)").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(CppProvider.detect(&ctx));
    }

    #[test]
    fn detects_makefile_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "all: app").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(CppProvider.detect(&ctx));
    }

    #[test]
    fn cmake_plan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CMakeLists.txt"), "project(myapp)").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = CppProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "cpp");
        assert_eq!(plan.stages.len(), 2);
        assert!(plan.stages[0].commands[0].run.contains("cmake"));
        // gcc:14 lacks cmake, so the build stage must provision it first
        assert!(plan.stages[0].commands[0].run.contains("install"));
        assert!(plan.stages[0].commands[0].run.contains("cmake -B build"));
        // the binary is copied to /app/app, so the runtime launches ./app (not
        // ./build/app, which only exists in the build stage)
        assert_eq!(plan.start_command.as_deref(), Some("./app"));
        assert_eq!(plan.stages[1].copy_from[0].dest, "/app/app");
        assert_eq!(plan.port, Some(8080));
    }

    #[test]
    fn cmake_binary_name_derived_from_add_executable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.10)\nproject(demo)\nadd_executable(server main.cpp)\n",
        )
        .unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = CppProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("./server"));
        assert_eq!(plan.stages[1].copy_from[0].src, "/app/build/server");
        assert_eq!(plan.stages[1].copy_from[0].dest, "/app/server");
    }

    #[test]
    fn cmake_falls_back_to_app_without_add_executable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CMakeLists.txt"), "project(myapp)").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = CppProvider.plan(&ctx).unwrap();
        assert_eq!(plan.start_command.as_deref(), Some("./app"));
    }

    #[test]
    fn makefile_plan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "all: app").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = CppProvider.plan(&ctx).unwrap();
        assert!(plan.stages[0].commands[0].run.contains("make"));
        assert_eq!(plan.start_command.as_deref(), Some("./app"));
    }
}
