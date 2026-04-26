package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.ExecuteTaskRequest;
import gradle.substrate.v1.ExecuteTaskResponse;

import org.gradle.api.Action;
import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.tasks.TaskContainer;
import org.gradle.api.tasks.TaskState;
import org.junit.Test;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

public class JvmTaskExecutionProviderAdapterTest {
    @Test
    public void executesRealizedTaskActions() {
        AtomicBoolean actionRan = new AtomicBoolean(false);
        AtomicBoolean didWork = new AtomicBoolean(false);
        Task task = task(":legacy", Collections.singletonList(t -> actionRan.set(true)), didWork);
        JvmTaskExecutionProviderAdapter adapter = new JvmTaskExecutionProviderAdapter(registryFor(task));

        ExecuteTaskResponse response = adapter.executeTask(request(":legacy"));

        assertTrue(response.getSuccess());
        assertEquals("EXECUTED", response.getOutcome());
        assertEquals("jvm_host_actions", response.getExecutionMode());
        assertTrue(actionRan.get());
        assertTrue(didWork.get());
    }

    @Test
    public void reportsUnsupportedWhenTaskIsMissing() {
        JvmTaskExecutionProviderAdapter adapter = new JvmTaskExecutionProviderAdapter(registryFor(null));

        ExecuteTaskResponse response = adapter.executeTask(request(":missing"));

        assertFalse(response.getSuccess());
        assertEquals("UNSUPPORTED", response.getOutcome());
        assertEquals("jvm_unsupported", response.getExecutionMode());
        assertEquals("Task not found: :missing", response.getErrorMessage());
    }

    private static ExecuteTaskRequest request(String taskPath) {
        return ExecuteTaskRequest.newBuilder()
            .setTaskPath(taskPath)
            .setTaskType("LegacyTask")
            .build();
    }

    private static Object registryFor(Task task) {
        return new FakeProjectStateRegistry(Collections.singletonList(new FakeProjectState(projectFor(task))));
    }

    private static Project projectFor(Task task) {
        TaskContainer tasks = proxy(TaskContainer.class, (proxy, method, args) -> {
            if (method.getName().equals("findByPath")) {
                return task;
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getTasks")) {
                return tasks;
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static Task task(String path, List<Action<? super Task>> actions, AtomicBoolean didWork) {
        TaskState state = state(false, null, false, false, false);
        return proxy(Task.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return path;
                case "getName":
                    return path.substring(path.lastIndexOf(':') + 1);
                case "getEnabled":
                    return true;
                case "getState":
                    return state;
                case "getActions":
                    return actions;
                case "setDidWork":
                    didWork.set((Boolean) args[0]);
                    return null;
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static TaskState state(
        boolean executed,
        Throwable failure,
        boolean noSource,
        boolean upToDate,
        boolean skipped
    ) {
        return proxy(TaskState.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getExecuted":
                    return executed;
                case "getFailure":
                    return failure;
                case "getNoSource":
                    return noSource;
                case "getUpToDate":
                    return upToDate;
                case "getSkipped":
                    return skipped;
                case "rethrowFailure":
                    if (failure instanceof RuntimeException) {
                        throw failure;
                    }
                    return null;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    @SuppressWarnings("unchecked")
    private static <T> T proxy(Class<T> type, InvocationHandler handler) {
        return (T) Proxy.newProxyInstance(
            type.getClassLoader(),
            new Class<?>[] {type},
            (proxy, method, args) -> {
                if (method.getDeclaringClass().equals(Object.class)) {
                    switch (method.getName()) {
                        case "toString":
                            return type.getSimpleName() + "Proxy";
                        case "hashCode":
                            return System.identityHashCode(proxy);
                        case "equals":
                            return proxy == args[0];
                        default:
                            return defaultValue(method.getReturnType());
                    }
                }
                return handler.invoke(proxy, method, args);
            }
        );
    }

    private static Object defaultValue(Class<?> returnType) {
        if (!returnType.isPrimitive()) {
            return null;
        }
        if (returnType.equals(boolean.class)) {
            return false;
        }
        if (returnType.equals(char.class)) {
            return '\0';
        }
        if (returnType.equals(void.class)) {
            return null;
        }
        return 0;
    }

    public static class FakeProjectStateRegistry {
        private final Collection<Object> projects;

        public FakeProjectStateRegistry(Collection<Object> projects) {
            this.projects = projects;
        }

        public Collection<Object> getAllProjects() {
            return projects;
        }
    }

    public static class FakeProjectState {
        private final Project project;

        public FakeProjectState(Project project) {
            this.project = project;
        }

        public boolean isCreated() {
            return true;
        }

        public Object getMutableModel() {
            return project;
        }
    }
}
