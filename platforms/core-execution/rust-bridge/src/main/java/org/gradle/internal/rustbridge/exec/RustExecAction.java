package org.gradle.internal.rustbridge.exec;

import gradle.substrate.v1.ExecSpawnRequest;
import gradle.substrate.v1.ExecSpawnResponse;
import org.gradle.api.logging.Logging;
import org.gradle.process.BaseExecSpec;
import org.gradle.process.ExecResult;
import org.gradle.process.ExecSpec;
import org.gradle.process.ProcessForkOptions;
import org.gradle.process.ProcessExecutionException;
import org.gradle.process.internal.ExecAction;
import org.gradle.process.internal.ExecHandleListener;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * ExecAction implementation that delegates to the Rust substrate daemon.
 * Used when the rust.exec feature flag is enabled.
 */
public class RustExecAction implements ExecAction {

    private static final Logger LOGGER = Logging.getLogger(RustExecAction.class);

    private final SubstrateClient client;
    private String executable;
    private List<String> args = Collections.emptyList();
    private File workingDir;
    private Map<String, Object> environment = new HashMap<>();
    private boolean ignoreExitValue;
    private InputStream stdin;
    private OutputStream stdout;
    private OutputStream stderr;
    private boolean redirectErrorStream;

    public RustExecAction(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public ExecResult execute() {
        if (client.isNoop()) {
            throw new RuntimeException("Rust exec is not available");
        }

        Map<String, String> envMap = new HashMap<>();
        for (Map.Entry<String, Object> entry : environment.entrySet()) {
            envMap.put(entry.getKey(), String.valueOf(entry.getValue()));
        }

        ExecSpawnResponse spawnResp = client.getExecStub().spawn(
            ExecSpawnRequest.newBuilder()
                .setCommand(executable)
                .addAllArgs(args)
                .setWorkingDir(workingDir != null ? workingDir.getAbsolutePath() : System.getProperty("user.dir"))
                .putAllEnvironment(envMap)
                .setRedirectErrorStream(redirectErrorStream)
                .build()
        );

        if (!spawnResp.getSuccess()) {
            throw new ProcessExecutionException("Failed to spawn process: " + spawnResp.getErrorMessage());
        }

        RustProcessHandle handle = new RustProcessHandle(client, spawnResp.getPid());

        try {
            handle.pumpOutput(stdout, redirectErrorStream ? stdout : stderr);
        } catch (java.io.IOException e) {
            LOGGER.debug("Error pumping process output", e);
        }

        int exitCode;
        try {
            exitCode = handle.waitForFinish().getExitValue();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            handle.abort();
            throw new ProcessExecutionException("Process interrupted", e);
        }

        if (!ignoreExitValue && exitCode != 0) {
            throw new ProcessExecutionException(
                "Process '" + executable + "' finished with non-zero exit value " + exitCode
            );
        }

        return new SimpleExecResult(exitCode);
    }

    // --- ExecAction ---

    @Override
    public ExecAction listener(ExecHandleListener listener) {
        // No-op: listener support not needed for substrate-backed execution
        return this;
    }

    // --- ExecSpec ---

    @Override
    public void setCommandLine(List<String> args) {
        if (args != null && !args.isEmpty()) {
            this.executable = args.get(0);
            this.args = args.subList(1, args.size());
        }
    }

    @Override
    public void setCommandLine(Object... args) {
        if (args != null && args.length > 0) {
            this.executable = args[0].toString();
            List<String> list = new ArrayList<>();
            for (int i = 1; i < args.length; i++) {
                list.add(args[i] != null ? args[i].toString() : null);
            }
            this.args = list;
        }
    }

    @Override
    public void setCommandLine(Iterable<?> args) {
        java.util.Iterator<?> it = args.iterator();
        if (it.hasNext()) {
            this.executable = it.next().toString();
            List<String> list = new ArrayList<>();
            while (it.hasNext()) {
                Object arg = it.next();
                list.add(arg != null ? arg.toString() : null);
            }
            this.args = list;
        }
    }

    @Override
    public ExecSpec commandLine(Object... args) {
        setCommandLine(args);
        return this;
    }

    @Override
    public ExecSpec commandLine(Iterable<?> args) {
        setCommandLine(args);
        return this;
    }

    @Override
    public ExecSpec args(Object... args) {
        List<String> list = new ArrayList<>(this.args);
        for (Object arg : args) {
            list.add(arg != null ? arg.toString() : null);
        }
        this.args = list;
        return this;
    }

    @Override
    public ExecSpec args(Iterable<?> args) {
        List<String> list = new ArrayList<>(this.args);
        for (Object arg : args) {
            list.add(arg != null ? arg.toString() : null);
        }
        this.args = list;
        return this;
    }

    @Override
    public ExecSpec setArgs(List<String> args) {
        this.args = args != null ? args : Collections.<String>emptyList();
        return this;
    }

    @Override
    public ExecSpec setArgs(Iterable<?> args) {
        List<String> list = new ArrayList<>();
        if (args != null) {
            for (Object arg : args) {
                list.add(arg != null ? arg.toString() : null);
            }
        }
        this.args = list;
        return this;
    }

    @Override
    public List<String> getArgs() {
        return args;
    }

    @Override
    public List<org.gradle.process.CommandLineArgumentProvider> getArgumentProviders() {
        return Collections.emptyList();
    }

    // --- BaseExecSpec ---

    @Override
    public BaseExecSpec setIgnoreExitValue(boolean ignoreExitValue) {
        this.ignoreExitValue = ignoreExitValue;
        return this;
    }

    @Override
    public boolean isIgnoreExitValue() {
        return ignoreExitValue;
    }

    @Override
    public BaseExecSpec setStandardInput(InputStream inputStream) {
        this.stdin = inputStream;
        return this;
    }

    @Override
    public InputStream getStandardInput() {
        return stdin;
    }

    @Override
    public BaseExecSpec setStandardOutput(OutputStream outputStream) {
        this.stdout = outputStream;
        return this;
    }

    @Override
    public OutputStream getStandardOutput() {
        return stdout;
    }

    @Override
    public BaseExecSpec setErrorOutput(OutputStream outputStream) {
        this.stderr = outputStream;
        return this;
    }

    @Override
    public OutputStream getErrorOutput() {
        return stderr;
    }

    @Override
    public List<String> getCommandLine() {
        List<String> commandLine = new ArrayList<>();
        if (executable != null) {
            commandLine.add(executable);
        }
        commandLine.addAll(args);
        return commandLine;
    }

    // --- ProcessForkOptions ---

    @Override
    public String getExecutable() {
        return executable;
    }

    @Override
    public void setExecutable(String executable) {
        this.executable = executable;
    }

    @Override
    public void setExecutable(Object executable) {
        this.executable = executable != null ? executable.toString() : null;
    }

    @Override
    public ProcessForkOptions executable(Object executable) {
        setExecutable(executable);
        return this;
    }

    @Override
    public File getWorkingDir() {
        return workingDir;
    }

    @Override
    public void setWorkingDir(File dir) {
        this.workingDir = dir;
    }

    @Override
    public void setWorkingDir(Object dir) {
        this.workingDir = dir != null ? new File(dir.toString()) : null;
    }

    @Override
    public ProcessForkOptions workingDir(Object dir) {
        setWorkingDir(dir);
        return this;
    }

    @Override
    public Map<String, Object> getEnvironment() {
        return environment;
    }

    @Override
    public void setEnvironment(Map<String, ?> environmentVariables) {
        this.environment = environmentVariables != null ? new HashMap<>(environmentVariables) : new HashMap<>();
    }

    @Override
    public ProcessForkOptions environment(Map<String, ?> environmentVariables) {
        if (environmentVariables != null) {
            this.environment.putAll(environmentVariables);
        }
        return this;
    }

    @Override
    public ProcessForkOptions environment(String name, Object value) {
        this.environment.put(name, value);
        return this;
    }

    @Override
    public ProcessForkOptions copyTo(ProcessForkOptions options) {
        options.setExecutable(this.executable);
        options.setArgs(this.args);
        options.setWorkingDir(this.workingDir);
        options.setEnvironment(this.environment);
        if (options instanceof BaseExecSpec) {
            ((BaseExecSpec) options).setIgnoreExitValue(this.ignoreExitValue);
        }
        if (options instanceof ExecSpec) {
            // nothing extra needed
        }
        return this;
    }

    /**
     * Simple ExecResult implementation for substrate-backed process execution.
     */
    private static class SimpleExecResult implements ExecResult {
        private final int exitValue;

        SimpleExecResult(int exitValue) {
            this.exitValue = exitValue;
        }

        @Override
        public int getExitValue() {
            return exitValue;
        }

        @Override
        public ExecResult assertNormalExitValue() throws ProcessExecutionException {
            if (exitValue != 0) {
                throw new ProcessExecutionException("Process finished with non-zero exit value " + exitValue);
            }
            return this;
        }

        @Override
        public ExecResult rethrowFailure() throws ProcessExecutionException {
            return this;
        }
    }
}
