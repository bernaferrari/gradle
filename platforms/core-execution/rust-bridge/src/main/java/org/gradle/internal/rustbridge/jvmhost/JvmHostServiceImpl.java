package org.gradle.internal.rustbridge.jvmhost;

import org.gradle.api.logging.Logging;
import org.slf4j.Logger;

/**
 * Implementation of the JVM Compatibility Host service.
 *
 * Allows the Rust daemon to call back into the JVM for operations that
 * require the JVM runtime (script evaluation, build model access, etc.).
 *
 * Initially only GetBuildEnvironment is fully implemented; other RPCs
 * return UNIMPLEMENTED status.
 */
public class JvmHostServiceImpl {

    private static final Logger LOGGER = Logging.getLogger(JvmHostServiceImpl.class);

    public JvmHostServiceImpl() {
    }

    /**
     * Returns build environment information from the JVM.
     */
    public String getJavaVersion() {
        return System.getProperty("java.version", "unknown");
    }

    public String getJavaHome() {
        return System.getProperty("java.home", "");
    }

    public String getGradleVersion() {
        try {
            Class<?> versionClass = Class.forName("org.gradle.util.GradleVersion");
            Object current = versionClass.getMethod("current").invoke(null);
            return current.toString();
        } catch (ReflectiveOperationException e) {
            LOGGER.debug("[substrate-jvmhost] Could not determine Gradle version", e);
            return "unknown";
        }
    }

    public String getOsName() {
        return System.getProperty("os.name", "unknown");
    }

    public String getOsArch() {
        return System.getProperty("os.arch", "unknown");
    }

    public int getAvailableProcessors() {
        return Runtime.getRuntime().availableProcessors();
    }

    public long getMaxMemoryBytes() {
        return Runtime.getRuntime().maxMemory();
    }
}
