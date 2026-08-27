use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct CppProvider;

enum BuildSystem {
    CMake,
    Make,
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

        let (build_cmd, start_cmd) = match build_system {
            BuildSystem::CMake => (
                // gcc:14 (buildpack-deps/Debian) ships gcc and make but NOT
                // cmake, so provision it before invoking the build
                "apt-get update && apt-get install -y --no-install-recommends cmake && cmake -B build && cmake --build build"
                    .to_string(),
                // both build systems copy the binary to /app/app (see copy_src
                // below), so the runtime launches ./app regardless; the CMake
                // binary is at /app/build/app in the BUILD stage only
                "./app".to_string(),
            ),
            BuildSystem::Make => ("make".to_string(), "./app".to_string()),
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

        let copy_src = match build_system {
            BuildSystem::CMake => "/app/build/app",
            BuildSystem::Make => "/app/app",
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
                src: copy_src.to_string(),
                dest: "/app/app".to_string(),
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
    fn makefile_plan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "all: app").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = CppProvider.plan(&ctx).unwrap();
        assert!(plan.stages[0].commands[0].run.contains("make"));
        assert_eq!(plan.start_command.as_deref(), Some("./app"));
    }
}
