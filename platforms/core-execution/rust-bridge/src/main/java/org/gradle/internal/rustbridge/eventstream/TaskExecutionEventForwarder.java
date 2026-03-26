package org.gradle.internal.rustbridge.eventstream;

import org.gradle.api.Task;
import org.gradle.api.execution.TaskExecutionListener;
import org.gradle.api.logging.Logging;
import org.gradle.api.tasks.TaskState;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Forwards JVM task execution events to the Rust substrate via gRPC.
 * Implements {@link TaskExecutionListener} to capture beforeExecute and afterExecute.
 */
public class TaskExecutionEventForwarder implements TaskExecutionListener {

    private static final Logger LOGGER = Logging.getLogger(TaskExecutionEventForwarder.class);

    private final RustBuildEventStreamClient eventStreamClient;
    private final ConcurrentHashMap<String, Long> taskStartTimes = new ConcurrentHashMap<>();

    public TaskExecutionEventForwarder(RustBuildEventStreamClient eventStreamClient) {
        this.eventStreamClient = eventStreamClient;
    }

    @Override
    public void beforeExecute(Task task) {
        taskStartTimes.put(task.getPath(), System.currentTimeMillis());

        try {
            Map<String, String> props = new HashMap<>();
            props.put("task_path", task.getPath());
            props.put("task_type", task.getClass().getSimpleName());

            eventStreamClient.sendBuildEvent(
                BuildIdHolder.getBuildId(),
                "jvm_task_start",
                "jvm-task-start-" + task.getPath(),
                props,
                task.getPath(),
                ""
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:lifecycle] beforeExecute event failed", e);
        }
    }

    @Override
    public void afterExecute(Task task, TaskState state) {
        Long startTime = taskStartTimes.remove(task.getPath());
        long durationMs = 0;
        if (startTime != null) {
            durationMs = System.currentTimeMillis() - startTime;
        }

        try {
            Map<String, String> props = new HashMap<>();
            props.put("task_path", task.getPath());
            props.put("task_type", task.getClass().getSimpleName());
            props.put("outcome", state.getOutcome() != null ? state.getOutcome().name() : "UNKNOWN");
            props.put("duration_ms", String.valueOf(durationMs));
            props.put("did_work", String.valueOf(state.getDidWork()));
            if (state.getSkipMessage() != null) {
                props.put("skip_message", state.getSkipMessage());
            }

            eventStreamClient.sendBuildEvent(
                BuildIdHolder.getBuildId(),
                "jvm_task_finish",
                "jvm-task-finish-" + task.getPath(),
                props,
                task.getPath(),
                ""
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:lifecycle] afterExecute event failed", e);
        }
    }
}
