package org.gradle.internal.rustbridge.exec;

import gradle.substrate.v1.ExecSpawnRequest;
import gradle.substrate.v1.ExecSpawnResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.io.File;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * ExecAction implementation that delegates to the Rust substrate daemon.
 * Used when the rust.exec feature flag is enabled.
 */
public class RustExecAction implements org.gradle.process.ExecAction {

    private static final Logger LOGGER = Logging.getLogger(RustExecAction.class);

    private final SubstrateClient client;
    private String executable;
    private List<String> args = java.util.Collections.emptyList();
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
    public org.gradle.process.ExecResult execute() {
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
            throw new RuntimeException("Failed to spawn process: " + spawnResp.getErrorMessage());
        }

        RustProcessHandle handle = new RustProcessHandle(client, spawnResp.getPid());

        try {
            handle.pumpOutput(stdout, redirectErrorStream ? stdout : stderr);
        } catch (IOException e) {
            LOGGER.debug("Error pumping process output", e);
        }

        int exitCode;
        try {
            exitCode = handle.waitFor();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            handle.abort();
            throw new org.gradle.process.internal.ExecException("Process interrupted", e);
        }

        if (!ignoreExitValue && exitCode != 0) {
            throw new org.gradle.process.internal.ExecException(
                "Process '" + executable + "' finished with non-zero exit value " + exitCode
            );
        }

        return new org.gradle.process.internal.DefaultExecResult(exitCode);
    }

    // --- ExecAction setters ---

    @Override
    public org.gradle.process.ExecAction setExecutable(Object executable) {
        this.executable = executable != null ? executable.toString() : null;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setArgs(Iterable<?> args) {
        java.util.List<String> list = new java.util.ArrayList<>();
        for (Object arg : args) {
            list.add(arg != null ? arg.toString() : null);
        }
        this.args = list;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setWorkingDir(Object dir) {
        this.workingDir = dir != null ? new File(dir.toString()) : null;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setEnvironment(Map<String, ?> environmentVariables) {
        this.environment = environmentVariables != null ? environmentVariables : new HashMap<>();
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setIgnoreExitValue(boolean ignoreExitValue) {
        this.ignoreExitValue = ignoreExitValue;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setStandardInput(InputStream inputStream) {
        this.stdin = inputStream;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setStandardOutput(OutputStream outputStream) {
        this.stdout = outputStream;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setErrorOutput(OutputStream outputStream) {
        this.stderr = outputStream;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction setRedirectErrorStream(boolean redirectErrorStream) {
        this.redirectErrorStream = redirectErrorStream;
        return this;
    }

    @Override
    public org.gradle.process.ExecAction copyTo(org.gradle.process.ExecAction execAction) {
        execAction.setExecutable(this.executable);
        execAction.setArgs(this.args);
        execAction.setWorkingDir(this.workingDir);
        execAction.setEnvironment(this.environment);
        execAction.setIgnoreExitValue(this.ignoreExitValue);
        execAction.setRedirectErrorStream(this.redirectErrorStream);
        return this;
    }
}
