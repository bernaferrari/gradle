package org.gradle.internal.rustbridge.eventstream;

import org.gradle.BuildListener;
import org.gradle.api.InitializationResult;
import org.gradle.api.invocation.Gradle;
import org.gradle.BuildResult;
import org.gradle.api.logging.Logging;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.Map;

/**
 * Forwards JVM build lifecycle events to the Rust substrate via gRPC.
 * Implements {@link BuildListener} to capture settingsEvaluated, projectsLoaded,
 * projectsEvaluated, and buildFinished events.
 */
public class BuildLifecycleEventForwarder implements BuildListener {

    private static final Logger LOGGER = Logging.getLogger(BuildLifecycleEventForwarder.class);

    private final RustBuildEventStreamClient eventStreamClient;

    public BuildLifecycleEventForwarder(RustBuildEventStreamClient eventStreamClient) {
        this.eventStreamClient = eventStreamClient;
    }

    @Override
    public void settingsEvaluated(org.gradle.api.initialization.Settings settings) {
        try {
            Map<String, String> props = new HashMap<>();
            String rootDir = settings.getSettingsDir().getAbsolutePath();
            props.put("settings_dir", rootDir);

            eventStreamClient.sendBuildEvent(
                BuildIdHolder.getBuildId(),
                "jvm_settings_evaluated",
                "jvm-settings-evaluated",
                props,
                "Settings",
                ""
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:lifecycle] settingsEvaluated event failed", e);
        }
    }

    @Override
    public void projectsLoaded(Gradle gradle) {
        try {
            Map<String, String> props = new HashMap<>();
            props.put("root_project", gradle.getRootProject().getName());
            props.put("project_count", String.valueOf(gradle.getRootProject().getAllprojects().size()));

            eventStreamClient.sendBuildEvent(
                BuildIdHolder.getBuildId(),
                "jvm_projects_loaded",
                "jvm-projects-loaded",
                props,
                "Projects",
                ""
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:lifecycle] projectsLoaded event failed", e);
        }
    }

    @Override
    public void projectsEvaluated(Gradle gradle) {
        try {
            Map<String, String> props = new HashMap<>();
            props.put("root_project", gradle.getRootProject().getName());
            props.put("project_count", String.valueOf(gradle.getRootProject().getAllprojects().size()));

            eventStreamClient.sendBuildEvent(
                BuildIdHolder.getBuildId(),
                "jvm_projects_evaluated",
                "jvm-projects-evaluated",
                props,
                "Projects",
                ""
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:lifecycle] projectsEvaluated event failed", e);
        }
    }

    @Override
    public void buildFinished(BuildResult result) {
        try {
            Map<String, String> props = new HashMap<>();
            props.put("action", result.getAction().name());
            if (result.getFailure() != null) {
                props.put("failure", result.getFailure().getMessage());
            }

            eventStreamClient.sendBuildEvent(
                BuildIdHolder.getBuildId(),
                "jvm_build_finished",
                "jvm-build-finished",
                props,
                "Build",
                ""
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:lifecycle] buildFinished event failed", e);
        } finally {
            BuildIdHolder.clear();
        }
    }
}
