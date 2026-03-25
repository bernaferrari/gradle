package org.gradle.internal.rustbridge.graph;

import org.gradle.api.execution.TaskExecutionGraph;
import org.gradle.api.execution.TaskExecutionGraphListener;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

/**
 * Shadow listener that captures task execution graph population events
 * and reports summary to the Rust substrate.
 */
public class TaskExecutionGraphShadowListener implements TaskExecutionGraphListener {

    private static final Logger LOGGER = Logging.getLogger(TaskExecutionGraphShadowListener.class);

    private final SubstrateClient client;
    private volatile int taskCount;
    private volatile long populateTimestampMs;

    public TaskExecutionGraphShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void graphPopulated(TaskExecutionGraph graph) {
        populateTimestampMs = System.currentTimeMillis();

        // Count tasks in the execution graph
        taskCount = graph.getAllTasks().size();

        // Report as a build event
        try {
            client.getBuildEventStreamStub().sendBuildEvent(
                gradle.substrate.v1.SendBuildEventRequest.newBuilder()
                    .setBuildId("")
                    .setEventType("task_graph_populated")
                    .setEventId("graph-populate")
                    .setDisplayName("Task graph populated")
                    .putProperties("task_count", String.valueOf(taskCount))
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:graph] failed to report graph populated: {}", e.getMessage());
        }
    }

    public int getTaskCount() {
        return taskCount;
    }

    public long getPopulateTimestampMs() {
        return populateTimestampMs;
    }
}
