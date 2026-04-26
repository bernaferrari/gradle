package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.ExecuteTaskRequest;
import gradle.substrate.v1.ExecuteTaskResponse;

import org.gradle.api.Action;
import org.gradle.api.Task;
import org.gradle.api.logging.Logging;
import org.gradle.api.tasks.StopExecutionException;
import org.gradle.api.tasks.TaskContainer;
import org.gradle.api.tasks.TaskState;
import org.gradle.internal.service.ServiceRegistry;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.Collection;
import java.util.Collections;
import java.util.List;

/**
 * Executes realized JVM task actions for the compatibility host.
 *
 * <p>This is intentionally narrower than Gradle's full task execution engine. The Rust daemon
 * owns DAG scheduling, while this adapter runs the already-realized public task actions in the
 * JVM when a task has no native Rust executor. Full Gradle execution semantics still require a
 * Gradle-owned {@code LocalTaskNode} and execution context, so this provider reports its
 * execution mode explicitly as {@code jvm_host_actions}.</p>
 */
public class JvmTaskExecutionProviderAdapter implements JvmHostServiceImpl.TaskExecutionProvider {
    private static final Logger LOGGER = Logging.getLogger(JvmTaskExecutionProviderAdapter.class);
    private static final String PROJECT_STATE_REGISTRY_CLASS = "org.gradle.api.internal.project.ProjectStateRegistry";

    @Nullable
    private final Object projectStateRegistry;

    public JvmTaskExecutionProviderAdapter(@Nullable Object projectStateRegistry) {
        this.projectStateRegistry = projectStateRegistry;
    }

    public static JvmTaskExecutionProviderAdapter fromServiceRegistry(ServiceRegistry services) {
        return new JvmTaskExecutionProviderAdapter(findProjectStateRegistry(services));
    }

    @Override
    public ExecuteTaskResponse executeTask(ExecuteTaskRequest request) {
        long startedAt = System.currentTimeMillis();
        String taskPath = request.getTaskPath();
        if (projectStateRegistry == null) {
            return unsupported("ProjectStateRegistry is unavailable");
        }

        Task task = findTask(taskPath);
        if (task == null) {
            return unsupported("Task not found: " + taskPath);
        }
        TaskState state = task.getState();
        if (state.getExecuted()) {
            return ExecuteTaskResponse.newBuilder()
                .setSuccess(state.getFailure() == null)
                .setOutcome(outcomeName(state))
                .setExecutionMode("jvm_host_actions")
                .setErrorMessage(failureMessage(state.getFailure()))
                .setDurationMs(elapsedSince(startedAt))
                .build();
        }

        if (!task.getEnabled()) {
            return ExecuteTaskResponse.newBuilder()
                .setSuccess(true)
                .setOutcome("SKIPPED")
                .setExecutionMode("jvm_host_actions")
                .setErrorMessage("Task is disabled")
                .setDurationMs(elapsedSince(startedAt))
                .build();
        }

        try {
            if (!isOnlyIfSatisfied(task)) {
                return ExecuteTaskResponse.newBuilder()
                    .setSuccess(true)
                    .setOutcome("SKIPPED")
                    .setExecutionMode("jvm_host_actions")
                    .setErrorMessage("Task onlyIf predicate was not satisfied")
                    .setDurationMs(elapsedSince(startedAt))
                    .build();
            }
        } catch (RuntimeException e) {
            return failed(startedAt, e);
        }

        List<Action<? super Task>> actions = task.getActions();
        if (actions.isEmpty()) {
            task.setDidWork(false);
            return ExecuteTaskResponse.newBuilder()
                .setSuccess(true)
                .setOutcome("EXECUTED")
                .setExecutionMode("jvm_host_actions")
                .setDurationMs(elapsedSince(startedAt))
                .build();
        }

        try {
            for (Action<? super Task> action : actions) {
                try {
                    action.execute(task);
                } catch (RuntimeException e) {
                    if (isStopActionException(e)) {
                        LOGGER.debug("[substrate-jvmhost] Task action stopped for {}", taskPath, e);
                        continue;
                    }
                    if (isStopExecutionException(e)) {
                        LOGGER.debug("[substrate-jvmhost] Task execution stopped for {}", taskPath, e);
                        return ExecuteTaskResponse.newBuilder()
                            .setSuccess(true)
                            .setOutcome("SKIPPED")
                            .setExecutionMode("jvm_host_actions")
                            .setErrorMessage(e.getMessage() == null ? "" : e.getMessage())
                            .setDurationMs(elapsedSince(startedAt))
                            .build();
                    }
                    throw e;
                }
            }
            task.setDidWork(true);
            return ExecuteTaskResponse.newBuilder()
                .setSuccess(true)
                .setOutcome("EXECUTED")
                .setExecutionMode("jvm_host_actions")
                .setDurationMs(elapsedSince(startedAt))
                .build();
        } catch (RuntimeException e) {
            return failed(startedAt, e);
        }
    }

    private static boolean isOnlyIfSatisfied(Task task) {
        try {
            Method method = task.getClass().getMethod("getOnlyIf");
            Object spec = method.invoke(task);
            if (spec == null) {
                return true;
            }
            Method isSatisfiedBy = spec.getClass().getMethod("isSatisfiedBy", Object.class);
            Object result = isSatisfiedBy.invoke(spec, task);
            return !(result instanceof Boolean) || (Boolean) result;
        } catch (NoSuchMethodException e) {
            return true;
        } catch (IllegalAccessException e) {
            throw new IllegalStateException("Failed to evaluate onlyIf for " + task.getPath(), e);
        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            if (cause instanceof RuntimeException) {
                throw (RuntimeException) cause;
            }
            throw new IllegalStateException("Failed to evaluate onlyIf for " + task.getPath(), cause);
        }
    }

    private static boolean isStopActionException(RuntimeException e) {
        return e.getClass().getName().equals("org.gradle.api.tasks.StopActionException");
    }

    private static boolean isStopExecutionException(RuntimeException e) {
        return e instanceof StopExecutionException
            || e.getClass().getName().equals("org.gradle.api.tasks.StopExecutionException");
    }

    @Nullable
    private Task findTask(String taskPath) {
        try {
            for (Object projectState : getAllProjects(projectStateRegistry)) {
                if (!booleanValue(projectState, "isCreated")) {
                    continue;
                }
                Object mutableModel = invoke(projectState, "getMutableModel");
                if (!(mutableModel instanceof org.gradle.api.Project)) {
                    continue;
                }
                TaskContainer tasks = ((org.gradle.api.Project) mutableModel).getTasks();
                Task task = tasks.findByPath(taskPath);
                if (task != null) {
                    return task;
                }
            }
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to find task {}", taskPath, e);
        }
        return null;
    }

    private static ExecuteTaskResponse unsupported(String message) {
        return ExecuteTaskResponse.newBuilder()
            .setSuccess(false)
            .setOutcome("UNSUPPORTED")
            .setExecutionMode("jvm_unsupported")
            .setErrorMessage(message)
            .build();
    }

    private static ExecuteTaskResponse failed(long startedAt, RuntimeException failure) {
        return ExecuteTaskResponse.newBuilder()
            .setSuccess(false)
            .setOutcome("FAILED")
            .setExecutionMode("jvm_host_actions")
            .setErrorMessage(failureMessage(failure))
            .setDurationMs(elapsedSince(startedAt))
            .build();
    }

    private static String outcomeName(TaskState state) {
        if (state.getFailure() != null) {
            return "FAILED";
        }
        if (state.getNoSource()) {
            return "NO_SOURCE";
        }
        if (state.getUpToDate()) {
            return "UP_TO_DATE";
        }
        if (state.getSkipped()) {
            return "SKIPPED";
        }
        if (state.getExecuted()) {
            return "EXECUTED";
        }
        return "UNKNOWN";
    }

    private static String failureMessage(@Nullable Throwable failure) {
        if (failure == null) {
            return "";
        }
        String message = failure.getMessage();
        return message == null ? failure.getClass().getName() : message;
    }

    private static long elapsedSince(long startedAt) {
        return Math.max(0L, System.currentTimeMillis() - startedAt);
    }

    @Nullable
    private static Object findProjectStateRegistry(ServiceRegistry services) {
        try {
            Class<?> registryType = Class.forName(PROJECT_STATE_REGISTRY_CLASS);
            return services.find(registryType);
        } catch (ClassNotFoundException e) {
            LOGGER.debug("[substrate-jvmhost] ProjectStateRegistry unavailable", e);
            return null;
        }
    }

    @SuppressWarnings("unchecked")
    private static Collection<Object> getAllProjects(@Nullable Object projectStateRegistry) {
        if (projectStateRegistry == null) {
            return Collections.emptyList();
        }
        return asCollection(invoke(projectStateRegistry, "getAllProjects"));
    }

    @SuppressWarnings("unchecked")
    private static Collection<Object> asCollection(@Nullable Object value) {
        if (value instanceof Collection) {
            return (Collection<Object>) value;
        }
        return Collections.emptyList();
    }

    private static boolean booleanValue(Object target, String methodName) {
        Object value = invoke(target, methodName);
        return value instanceof Boolean && (Boolean) value;
    }

    @Nullable
    private static Object invoke(Object target, String methodName) {
        try {
            Method method = target.getClass().getMethod(methodName);
            return method.invoke(target);
        } catch (NoSuchMethodException | IllegalAccessException e) {
            throw new IllegalStateException("Failed to invoke " + methodName + " on " + target.getClass().getName(), e);
        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            if (cause instanceof RuntimeException) {
                throw (RuntimeException) cause;
            }
            throw new IllegalStateException("Failed to invoke " + methodName + " on " + target.getClass().getName(), cause);
        }
    }
}
