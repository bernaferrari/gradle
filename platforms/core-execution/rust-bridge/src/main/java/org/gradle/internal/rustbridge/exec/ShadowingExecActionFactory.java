package org.gradle.internal.rustbridge.exec;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.process.internal.ExecAction;
import org.gradle.process.internal.ExecActionFactory;
import org.gradle.process.internal.JavaExecAction;
import org.gradle.process.internal.ExecHandleListener;
import org.gradle.process.ExecResult;
import org.gradle.process.ProcessExecutionException;
import org.slf4j.Logger;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.List;
import java.util.Map;

/**
 * An {@link ExecActionFactory} that creates exec actions that shadow against Rust.
 *
 * <p>In shadow mode, each exec action runs both Java and Rust implementations,
 * comparing exit codes. Java result is always authoritative.</p>
 *
 * <p>In authoritative mode, Rust handles execution with Java fallback.</p>
 */
public class ShadowingExecActionFactory implements ExecActionFactory {

    private static final Logger LOGGER = Logging.getLogger(ShadowingExecActionFactory.class);

    private final ExecActionFactory javaDelegate;
    private final SubstrateClient client;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingExecActionFactory(
        ExecActionFactory javaDelegate,
        SubstrateClient client,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.javaDelegate = javaDelegate;
        this.client = client;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    @Override
    public ExecAction newExecAction() {
        if (client.isNoop()) {
            return javaDelegate.newExecAction();
        }

        ExecAction javaAction = javaDelegate.newExecAction();
        return new ShadowingExecAction(javaAction, new RustExecAction(client), mismatchReporter, authoritative);
    }

    @Override
    public JavaExecAction newJavaExecAction() {
        return javaDelegate.newJavaExecAction();
    }

    /**
     * An {@link ExecAction} that shadows execution between Java and Rust.
     */
    private static class ShadowingExecAction implements ExecAction {

        private final ExecAction javaDelegate;
        private final RustExecAction rustDelegate;
        private final HashMismatchReporter mismatchReporter;
        private final boolean authoritative;

        ShadowingExecAction(
            ExecAction javaDelegate,
            RustExecAction rustDelegate,
            HashMismatchReporter mismatchReporter,
            boolean authoritative
        ) {
            this.javaDelegate = javaDelegate;
            this.rustDelegate = rustDelegate;
            this.mismatchReporter = mismatchReporter;
            this.authoritative = authoritative;
        }

        @Override
        public ExecResult execute() throws ProcessExecutionException {
            if (authoritative) {
                return executeAuthoritative();
            }
            return executeShadow();
        }

        private ExecResult executeShadow() {
            // Always run Java first (authoritative result)
            ExecResult javaResult;
            try {
                javaResult = javaDelegate.execute();
            } catch (ProcessExecutionException e) {
                mismatchReporter.reportJavaError("exec:" + getCommandDescription(), e);
                try {
                    syncToRust();
                    rustDelegate.execute();
                } catch (Exception ignored) {
                    // Rust failed too, that's fine in shadow mode
                }
                throw e;
            }

            // Shadow: run Rust and compare exit code
            try {
                syncToRust();
                ExecResult rustResult = rustDelegate.execute();
                int javaExitCode = javaResult.getExitValue();
                int rustExitCode = rustResult.getExitValue();

                if (javaExitCode == rustExitCode) {
                    mismatchReporter.reportMatch();
                } else {
                    mismatchReporter.reportMismatch(
                        "exec:" + getCommandDescription(),
                        String.valueOf(javaExitCode),
                        String.valueOf(rustExitCode)
                    );
                    LOGGER.warn("[substrate:exec] exit code mismatch for {}: java={} rust={}",
                        getCommandDescription(), javaExitCode, rustExitCode);
                }
            } catch (Exception e) {
                mismatchReporter.reportRustError("exec:" + getCommandDescription(), e);
                LOGGER.debug("[substrate:exec] shadow execution failed for {}", getCommandDescription(), e);
            }

            return javaResult;
        }

        private ExecResult executeAuthoritative() throws ProcessExecutionException {
            try {
                syncToRust();
                return rustDelegate.execute();
            } catch (Exception e) {
                mismatchReporter.reportRustError("exec:" + getCommandDescription(), e);
                LOGGER.debug("[substrate:exec] authoritative execution failed, falling back to Java for {}",
                    getCommandDescription(), e);
                return javaDelegate.execute();
            }
        }

        private void syncToRust() {
            try {
                rustDelegate.setCommandLine(javaDelegate.getCommandLine());
                rustDelegate.setWorkingDir(javaDelegate.getWorkingDir());
                rustDelegate.setEnvironment(javaDelegate.getEnvironment());
                rustDelegate.setIgnoreExitValue(javaDelegate.isIgnoreExitValue());
                if (javaDelegate.getStandardOutput() != null) {
                    rustDelegate.setStandardOutput(javaDelegate.getStandardOutput());
                }
                if (javaDelegate.getErrorOutput() != null) {
                    rustDelegate.setErrorOutput(javaDelegate.getErrorOutput());
                }
            } catch (Exception e) {
                LOGGER.debug("[substrate:exec] failed to sync exec config to Rust", e);
            }
        }

        private String getCommandDescription() {
            try {
                return String.valueOf(javaDelegate.getCommandLine());
            } catch (Exception e) {
                return "unknown";
            }
        }

        // --- Delegate all ExecSpec setters to Java ---

        @Override public ExecAction setCommandLine(Object... commandLine) { javaDelegate.setCommandLine(commandLine); return this; }
        @Override public ExecAction setCommandLine(String... commandLine) { javaDelegate.setCommandLine(commandLine); return this; }
        @Override public ExecAction setCommandLine(List<String> commandLine) { javaDelegate.setCommandLine(commandLine); return this; }
        @Override public ExecAction setArgs(Iterable<?> arguments) { javaDelegate.setArgs(arguments); return this; }
        @Override public ExecAction setArgs(Object... arguments) { javaDelegate.setArgs(arguments); return this; }
        @Override public ExecAction args(Object... arguments) { javaDelegate.args(arguments); return this; }
        @Override public ExecAction args(Iterable<?> arguments) { javaDelegate.args(arguments); return this; }
        @Override public ExecAction setIgnoreExitValue(boolean ignoreExitValue) { javaDelegate.setIgnoreExitValue(ignoreExitValue); return this; }
        @Override public ExecAction setStandardInput(InputStream input) { javaDelegate.setStandardInput(input); return this; }
        @Override public ExecAction setStandardOutput(OutputStream output) { javaDelegate.setStandardOutput(output); return this; }
        @Override public ExecAction setErrorOutput(OutputStream output) { javaDelegate.setErrorOutput(output); return this; }
        @Override public ExecAction setExecutable(Object executable) { javaDelegate.setExecutable(executable); return this; }
        @Override public ExecAction setWorkingDir(File dir) { javaDelegate.setWorkingDir(dir); return this; }
        @Override public ExecAction setEnvironment(Map<String, ?> environmentVariables) { javaDelegate.setEnvironment(environmentVariables); return this; }
        @Override public ExecAction environment(String name, Object value) { javaDelegate.environment(name, value); return this; }
        @Override public ExecAction environment(Map<String, ?> environmentVariables) { javaDelegate.environment(environmentVariables); return this; }
        @Override public ExecAction copyTo(org.gradle.process.ExecSpec destination) { javaDelegate.copyTo(destination); return this; }
        @Override public List<String> getCommandLine() { return javaDelegate.getCommandLine(); }
        @Override public List<String> getArgs() { return javaDelegate.getArgs(); }
        @Override public File getWorkingDir() { return javaDelegate.getWorkingDir(); }
        @Override public Map<String, Object> getEnvironment() { return javaDelegate.getEnvironment(); }
        @Override public boolean isIgnoreExitValue() { return javaDelegate.isIgnoreExitValue(); }
        @Override public InputStream getStandardInput() { return javaDelegate.getStandardInput(); }
        @Override public OutputStream getStandardOutput() { return javaDelegate.getStandardOutput(); }
        @Override public OutputStream getErrorOutput() { return javaDelegate.getErrorOutput(); }
        @Override public ExecAction listener(ExecHandleListener listener) { javaDelegate.listener(listener); return this; }
    }
}
