package org.gradle.internal.rustbridge;

import org.gradle.api.logging.Logging;
import org.gradle.internal.event.ListenerManager;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;

/**
 * Registers build lifecycle callbacks without taking a compile-time dependency on :core.
 */
public final class RootBuildLifecycleBridge {

    private static final Logger LOGGER = Logging.getLogger(RootBuildLifecycleBridge.class);
    private static final String ROOT_BUILD_LIFECYCLE_LISTENER = "org.gradle.initialization.RootBuildLifecycleListener";

    private RootBuildLifecycleBridge() {
    }

    public interface Callbacks {
        void afterStart();
        void beforeComplete(@Nullable Throwable failure);
    }

    public static Object createListener(Callbacks callbacks) {
        try {
            Class<?> listenerType = Class.forName(ROOT_BUILD_LIFECYCLE_LISTENER);
            InvocationHandler handler = new RootBuildLifecycleInvocationHandler(callbacks);
            return Proxy.newProxyInstance(
                listenerType.getClassLoader(),
                new Class<?>[]{listenerType},
                handler
            );
        } catch (ClassNotFoundException e) {
            LOGGER.debug("[substrate] RootBuildLifecycleListener unavailable, build lifecycle bridge disabled", e);
            return callbacks;
        }
    }

    public static void register(ListenerManager listenerManager, Callbacks callbacks) {
        Object listener = createListener(callbacks);
        if (listener != callbacks) {
            listenerManager.addListener(listener);
        }
    }

    private static final class RootBuildLifecycleInvocationHandler implements InvocationHandler {
        private final Callbacks callbacks;

        private RootBuildLifecycleInvocationHandler(Callbacks callbacks) {
            this.callbacks = callbacks;
        }

        @Override
        public Object invoke(Object proxy, Method method, Object[] args) {
            String name = method.getName();
            if ("afterStart".equals(name)) {
                callbacks.afterStart();
                return null;
            }
            if ("beforeComplete".equals(name)) {
                Throwable failure = args != null && args.length > 0 && args[0] instanceof Throwable
                    ? (Throwable) args[0]
                    : null;
                callbacks.beforeComplete(failure);
                return null;
            }
            if ("toString".equals(name)) {
                return callbacks.toString();
            }
            if ("hashCode".equals(name)) {
                return callbacks.hashCode();
            }
            if ("equals".equals(name)) {
                return proxy == (args == null || args.length == 0 ? null : args[0]);
            }
            throw new UnsupportedOperationException("Unsupported RootBuildLifecycleListener method: " + method);
        }
    }
}
