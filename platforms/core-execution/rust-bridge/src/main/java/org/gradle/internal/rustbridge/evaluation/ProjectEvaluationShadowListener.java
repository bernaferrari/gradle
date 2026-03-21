package org.gradle.internal.rustbridge.evaluation;

import org.gradle.api.Project;
import org.gradle.api.ProjectEvaluationListener;
import org.gradle.api.ProjectState;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Shadow listener that captures project evaluation events and reports
 * timing to the Rust substrate. Registered via ListenerManager.
 */
public class ProjectEvaluationShadowListener implements ProjectEvaluationListener {

    private static final Logger LOGGER = Logging.getLogger(ProjectEvaluationShadowListener.class);

    private final SubstrateClient client;
    private final ConcurrentHashMap<String, Long> evaluationStartTimes = new ConcurrentHashMap<>();
    private final AtomicInteger evaluatedCount = new AtomicInteger(0);
    private final AtomicInteger failedCount = new AtomicInteger(0);
    private final AtomicLong totalEvalDurationMs = new AtomicLong(0);
    private final AtomicLong slowestEvalMs = new AtomicLong(0);
    private volatile String slowestProject = "";

    public ProjectEvaluationShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void beforeEvaluate(Project project) {
        String path = project.getPath();
        evaluationStartTimes.put(path, System.currentTimeMillis());
    }

    @Override
    public void afterEvaluate(Project project, ProjectState state) {
        String path = project.getPath();
        Long startTime = evaluationStartTimes.remove(path);
        if (startTime == null) {
            return;
        }

        long durationMs = System.currentTimeMillis() - startTime;
        evaluatedCount.incrementAndGet();
        totalEvalDurationMs.addAndGet(durationMs);

        if (state != null && state.getFailure() != null) {
            failedCount.incrementAndGet();
        }

        // Track slowest project
        long currentSlowest = slowestEvalMs.get();
        if (durationMs > currentSlowest) {
            slowestEvalMs.compareAndSet(currentSlowest, durationMs);
            slowestProject = path;
        }

        // Report as a build event
        try {
            client.getBuildEventStreamStub().sendBuildEvent(
                gradle.substrate.v1.SendBuildEventRequest.newBuilder()
                    .setBuildId("")
                    .setEventType("project_evaluated")
                    .setEventId("eval-" + path)
                    .setDisplayName("Evaluate " + path)
                    .putProperties("project", path)
                    .putProperties("duration_ms", String.valueOf(durationMs))
                    .putProperties("failed", String.valueOf(state != null && state.getFailure() != null))
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:evaluation] failed to report evaluation: {}", e.getMessage());
        }
    }

    public int getEvaluatedCount() {
        return evaluatedCount.get();
    }

    public int getFailedCount() {
        return failedCount.get();
    }

    public long getTotalEvalDurationMs() {
        return totalEvalDurationMs.get();
    }

    public long getSlowestEvalMs() {
        return slowestEvalMs.get();
    }

    public String getSlowestProject() {
        return slowestProject;
    }
}
