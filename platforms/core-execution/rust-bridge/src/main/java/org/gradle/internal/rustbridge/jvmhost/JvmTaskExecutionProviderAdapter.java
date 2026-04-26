package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.ExecuteTaskRequest;
import gradle.substrate.v1.ExecuteTaskResponse;

import org.gradle.api.Task;
import org.gradle.api.logging.Logging;
import org.gradle.api.tasks.TaskContainer;
import org.gradle.api.tasks.TaskState;
import org.gradle.internal.service.ServiceRegistry;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.Collection;
import java.util.Collections;

/**
 * Executes realized JVM task actions for the compatibility host.
 *
 * <p>The Rust daemon owns DAG scheduling, while this adapter delegates a realized legacy task
 * back into Gradle's JVM execution services. If those internal services are not visible from the
 * compatibility host, the task is reported as unsupported instead of executing a degraded fallback.
 * </p>
 */
public class JvmTaskExecutionProviderAdapter implements JvmHostServiceImpl.TaskExecutionProvider {
    private static final Logger LOGGER = Logging.getLogger(JvmTaskExecutionProviderAdapter.class);
    private static final String PROJECT_STATE_REGISTRY_CLASS = "org.gradle.api.internal.project.ProjectStateRegistry";
    private static final String TASK_INTERNAL_CLASS = "org.gradle.api.internal.TaskInternal";
    private static final String PROJECT_INTERNAL_CLASS = "org.gradle.api.internal.project.ProjectInternal";
    private static final String TASK_NODE_FACTORY_CLASS = "org.gradle.execution.plan.TaskNodeFactory";
    private static final String PROJECT_EXECUTION_SERVICE_REGISTRY_CLASS = "org.gradle.execution.ProjectExecutionServiceRegistry";
    private static final String DEFAULT_NODE_EXECUTOR_CLASS = "org.gradle.execution.plan.DefaultNodeExecutor";
    private static final String NODE_CLASS = "org.gradle.execution.plan.Node";
    private static final String NODE_EXECUTION_CONTEXT_CLASS = "org.gradle.api.internal.tasks.NodeExecutionContext";

    @Nullable
    private final ServiceRegistry services;

    @Nullable
    private final Object projectStateRegistry;

    public JvmTaskExecutionProviderAdapter(@Nullable Object projectStateRegistry) {
        this(null, projectStateRegistry);
    }

    private JvmTaskExecutionProviderAdapter(@Nullable ServiceRegistry services, @Nullable Object projectStateRegistry) {
        this.services = services;
        this.projectStateRegistry = projectStateRegistry;
    }

    public static JvmTaskExecutionProviderAdapter fromServiceRegistry(ServiceRegistry services) {
        return new JvmTaskExecutionProviderAdapter(services, findProjectStateRegistry(services));
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

        ExecuteTaskResponse gradleEngineResponse = executeWithGradleTaskEngine(task, startedAt);
        if (gradleEngineResponse != null) {
            return gradleEngineResponse;
        }
        return unsupported("Gradle task execution services are unavailable for task: " + taskPath);
    }

    @Nullable
    private ExecuteTaskResponse executeWithGradleTaskEngine(Task task, long startedAt) {
        if (services == null) {
            return null;
        }
        Object executionServices = null;
        try {
            Class<?> taskInternalType = Class.forName(TASK_INTERNAL_CLASS);
            if (!taskInternalType.isInstance(task)) {
                return null;
            }
            Object taskNodeFactory = findService(TASK_NODE_FACTORY_CLASS);
            if (taskNodeFactory == null) {
                return null;
            }

            Object node = invoke(taskNodeFactory, "getOrCreateNode", new Class<?>[] {Task.class}, task);
            executionServices = Class.forName(PROJECT_EXECUTION_SERVICE_REGISTRY_CLASS)
                .getConstructor(ServiceRegistry.class)
                .newInstance(services);
            Object executionContext = invoke(
                executionServices,
                "forProject",
                new Class<?>[] {Class.forName(PROJECT_INTERNAL_CLASS)},
                task.getProject()
            );
            Object nodeExecutor = Class.forName(DEFAULT_NODE_EXECUTOR_CLASS).getConstructor().newInstance();
            executePrepareNodeIfPresent(nodeExecutor, node, executionContext);
            Object executed = invoke(
                nodeExecutor,
                "execute",
                new Class<?>[] {Class.forName(NODE_CLASS), Class.forName(NODE_EXECUTION_CONTEXT_CLASS)},
                node,
                executionContext
            );
            if (!Boolean.TRUE.equals(executed)) {
                return null;
            }
            TaskState state = task.getState();
            return ExecuteTaskResponse.newBuilder()
                .setSuccess(state.getFailure() == null)
                .setOutcome(outcomeName(state))
                .setExecutionMode("jvm_gradle_task_executer")
                .setErrorMessage(failureMessage(state.getFailure()))
                .setDurationMs(elapsedSince(startedAt))
                .build();
        } catch (ClassNotFoundException | NoSuchMethodException e) {
            LOGGER.debug("[substrate-jvmhost] Gradle task engine is unavailable", e);
            return null;
        } catch (ReflectiveOperationException | RuntimeException e) {
            return failed(startedAt, unwrapRuntimeFailure(e), "jvm_gradle_task_executer");
        } finally {
            closeQuietly(executionServices);
        }
    }

    private static void executePrepareNodeIfPresent(Object nodeExecutor, Object node, Object executionContext) throws ReflectiveOperationException {
        Object prepareNode = invoke(node, "getPrepareNode", new Class<?>[0]);
        if (prepareNode == null) {
            return;
        }
        invoke(
            nodeExecutor,
            "execute",
            new Class<?>[] {Class.forName(NODE_CLASS), Class.forName(NODE_EXECUTION_CONTEXT_CLASS)},
            prepareNode,
            executionContext
        );
        Object prepareFailure = invoke(prepareNode, "getNodeFailure", new Class<?>[0]);
        if (prepareFailure instanceof RuntimeException) {
            throw (RuntimeException) prepareFailure;
        }
        if (prepareFailure instanceof Exception) {
            throw new IllegalStateException("Task mutation resolution failed", (Exception) prepareFailure);
        }
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

    private static ExecuteTaskResponse failed(long startedAt, RuntimeException failure, String executionMode) {
        return ExecuteTaskResponse.newBuilder()
            .setSuccess(false)
            .setOutcome("FAILED")
            .setExecutionMode(executionMode)
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

    @Nullable
    private Object findService(String className) throws ClassNotFoundException {
        if (services == null) {
            return null;
        }
        try {
            return services.find(Class.forName(className));
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Gradle task engine service {} is unavailable", className, e);
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
        return invoke(target, methodName, new Class<?>[0]);
    }

    @Nullable
    private static Object invoke(Object target, String methodName, Class<?>[] parameterTypes, Object... args) {
        try {
            Method method = target.getClass().getMethod(methodName, parameterTypes);
            return method.invoke(target, args);
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

    private static RuntimeException unwrapRuntimeFailure(Exception failure) {
        if (failure instanceof RuntimeException) {
            return (RuntimeException) failure;
        }
        return new IllegalStateException(failure);
    }

    private static void closeQuietly(@Nullable Object value) {
        if (!(value instanceof AutoCloseable)) {
            return;
        }
        try {
            ((AutoCloseable) value).close();
        } catch (Exception e) {
            LOGGER.debug("[substrate-jvmhost] Failed to close project execution services", e);
        }
    }
}
