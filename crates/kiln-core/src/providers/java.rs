use crate::detect::AppContext;
use crate::error::Result;
use crate::plan::{BuildPlan, Command, CopyDirective, CopyFrom, Stage};
use crate::providers::Provider;

pub struct JavaProvider;

enum BuildTool {
    Maven,
    Gradle,
}

impl BuildTool {
    const fn build_image(&self) -> &'static str {
        match self {
            Self::Maven => "maven:3-eclipse-temurin-21",
            Self::Gradle => "gradle:8-jdk21",
        }
    }

    const fn build_command(&self) -> &'static str {
        match self {
            Self::Maven => "mvn package -DskipTests",
            Self::Gradle => "gradle build -x test",
        }
    }

    const fn cache_dir(&self) -> &'static str {
        match self {
            Self::Maven => "/root/.m2",
            Self::Gradle => "/root/.gradle",
        }
    }

    const fn jar_source(&self) -> &'static str {
        match self {
            Self::Maven => "/app/target/*.jar",
            Self::Gradle => "/app/build/libs/*.jar",
        }
    }
}

impl JavaProvider {
    fn detect_build_tool(ctx: &AppContext) -> Option<BuildTool> {
        if ctx.has_file("pom.xml") {
            Some(BuildTool::Maven)
        } else if ctx.has_file("build.gradle") || ctx.has_file("build.gradle.kts") {
            Some(BuildTool::Gradle)
        } else {
            None
        }
    }
}

impl Provider for JavaProvider {
    fn name(&self) -> &'static str {
        "java"
    }

    fn detect(&self, ctx: &AppContext) -> bool {
        ctx.has_file("pom.xml") || ctx.has_file("build.gradle") || ctx.has_file("build.gradle.kts")
    }

    fn plan(&self, ctx: &AppContext) -> Result<BuildPlan> {
        let tool = Self::detect_build_tool(ctx).unwrap_or(BuildTool::Maven);

        let build_stage = Stage {
            name: "build".to_string(),
            base_image: tool.build_image().to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![CopyDirective {
                src: ".".to_string(),
                dest: ".".to_string(),
            }],
            copy_from: vec![],
            commands: vec![
                Command {
                    run: tool.build_command().to_string(),
                    cache_mounts: vec![tool.cache_dir().to_string()],
                },
                // Gradle (and Spring Boot) emit several jars in the output dir:
                // the executable one plus `-plain`/`-sources`/`-javadoc`
                // variants. A plain `cp *.jar` would fail (multiple sources into
                // a non-directory) or grab the non-runnable plain jar, so pick
                // the largest jar that is not one of those variants.
                Command {
                    run: format!(
                        "cp \"$(ls -S {} | grep -Ev -- '-(plain|sources|javadoc)\\.jar$' | head -1)\" /app/app.jar",
                        tool.jar_source()
                    ),
                    cache_mounts: vec![],
                },
            ],
        };

        let runtime_stage = Stage {
            name: "runtime".to_string(),
            base_image: "eclipse-temurin:21-jre".to_string(),
            workdir: "/app".to_string(),
            copy_files: vec![],
            copy_from: vec![CopyFrom {
                stage: "build".to_string(),
                src: "/app/app.jar".to_string(),
                dest: "/app/app.jar".to_string(),
            }],
            commands: vec![],
        };

        Ok(BuildPlan {
            provider: "java".to_string(),
            stages: vec![build_stage, runtime_stage],
            start_command: Some("java -jar app.jar".to_string()),
            port: Some(8080),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_maven() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pom.xml"), "<project/>").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(JavaProvider.detect(&ctx));
    }

    #[test]
    fn detects_gradle() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.gradle"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        assert!(JavaProvider.detect(&ctx));
    }

    #[test]
    fn maven_plan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pom.xml"), "<project/>").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = JavaProvider.plan(&ctx).unwrap();
        assert_eq!(plan.provider, "java");
        assert_eq!(plan.stages.len(), 2);
        assert!(plan.stages[0].commands[0].run.contains("mvn"));
        assert!(
            plan.stages[0].commands[0]
                .cache_mounts
                .contains(&"/root/.m2".to_string())
        );
        assert_eq!(plan.start_command.as_deref(), Some("java -jar app.jar"));
        assert_eq!(plan.port, Some(8080));
    }

    #[test]
    fn gradle_plan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.gradle.kts"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = JavaProvider.plan(&ctx).unwrap();
        assert!(plan.stages[0].commands[0].run.contains("gradle"));
        assert!(
            plan.stages[0].commands[0]
                .cache_mounts
                .contains(&"/root/.gradle".to_string())
        );
    }

    #[test]
    fn jar_copy_excludes_plain_and_aux_jars() {
        // A Spring Boot Gradle build emits app.jar AND app-plain.jar; the copy
        // must select the executable jar, not fail on multiple sources or grab
        // the non-runnable plain jar.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.gradle"), "").unwrap();
        let ctx = AppContext::new(dir.path()).unwrap();
        let plan = JavaProvider.plan(&ctx).unwrap();
        let copy = &plan.stages[0].commands[1].run;
        assert!(copy.contains("/app/build/libs/*.jar"));
        assert!(copy.contains("-(plain|sources|javadoc)"));
        assert!(copy.ends_with("/app/app.jar"));
    }
}
