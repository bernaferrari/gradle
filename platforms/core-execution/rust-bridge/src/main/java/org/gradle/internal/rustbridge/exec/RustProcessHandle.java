package org.gradle.internal.rustbridge.exec;

import gradle.substrate.v1.ExecOutputChunk;
import gradle.substrate.v1.ExecOutputRequest;
import gradle.substrate.v1.ExecWaitRequest;
import gradle.substrate.v1.ExecWaitResponse;
import gradle.substrate.v1.ExecKillTreeRequest;
import io.grpc.stub.StreamObserver;
import org.gradle.process.ExecResult;
import org.gradle.process.internal.ExecHandle;
import org.gradle.process.internal.ExecHandleListener;
import org.gradle.process.internal.ExecHandleState;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.jspecify.annotations.Nullable;

import java.io.File;
import java.io.IOException;
import java.io.OutputStream;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/**
 * Process handle backed by the Rust substrate daemon.
 * Delegates spawn/wait/signal/kill to the Rust ExecService via gRPC.
 */
public class RustProcessHandle implements ExecHandle {

    private final SubstrateClient client;
    private final int pid;
    private final String command;
    private final List<String> arguments;
    private final File directory;
    private final Map<String, String> envSnapshot;
    private volatile ExecHandleState state = ExecHandleState.INIT;
    private volatile int exitCode = -1;
    private final List<ExecHandleListener> listeners = new CopyOnWriteArrayList<>();

    public RustProcessHandle(SubstrateClient client, int pid) {
        this(client, pid, "unknown", Collections.<String>emptyList(), null, Collections.<String, String>emptyMap());
    }

    public RustProcessHandle(SubstrateClient client, int pid, String command, List<String> arguments,
                             File directory, Map<String, String> envSnapshot) {
        this.client = client;
        this.pid = pid;
        this.command = command;
        this.arguments = arguments;
        this.directory = directory;
        this.envSnapshot = envSnapshot;
    }

    // --- ExecHandle ---

    @Override
    public File getDirectory() {
        return directory;
    }

    @Override
    public String getCommand() {
        return command;
    }

    @Override
    public List<String> getArguments() {
        return arguments;
    }

    @Override
    public Map<String, String> getEnvironment() {
        return envSnapshot;
    }

    @Override
    public ExecHandle start() {
        state = ExecHandleState.STARTED;
        for (ExecHandleListener listener : listeners) {
            listener.executionStarted(this);
        }
        return this;
    }

    @Override
    public void removeStartupContext() {
        // No-op for substrate-backed handles
    }

    @Override
    public ExecHandleState getState() {
        return state;
    }

    @Override
    public void sendSignal(int signal) {
        // Not supported for substrate-backed handles
        throw new UnsupportedOperationException("Signal sending not supported for substrate-backed process handles");
    }

    @Override
    public void abort() {
        try {
            client.getExecStub().killTree(
                ExecKillTreeRequest.newBuilder().setPid(pid).build()
            );
        } catch (Exception e) {
            // Ignore errors during abort
        }
        state = ExecHandleState.ABORTED;
        exitCode = -1;
    }

    @Override
    public ExecResult waitForFinish() {
        if (state.isTerminal()) {
            return createResult();
        }

        try {
            ExecWaitResponse response = client.getExecStub().waitExec(
                ExecWaitRequest.newBuilder().setPid(pid).build()
            );
            exitCode = response.getExitCode();
            state = exitCode == 0 ? ExecHandleState.SUCCEEDED : ExecHandleState.FAILED;
        } catch (Exception e) {
            state = ExecHandleState.FAILED;
            exitCode = -1;
        }

        ExecResult result = createResult();
        for (ExecHandleListener listener : listeners) {
            listener.executionFinished(this, result);
        }
        return result;
    }

    @Override
    @Nullable
    public ExecResult getExecResult() {
        if (!state.isTerminal()) {
            return null;
        }
        return createResult();
    }

    @Override
    public void addListener(ExecHandleListener listener) {
        listeners.add(listener);
    }

    @Override
    public void removeListener(ExecHandleListener listener) {
        listeners.remove(listener);
    }

    @Override
    public String getDisplayName() {
        return "RustProcessHandle[pid=" + pid + ", command=" + command + "]";
    }

    // --- Public API ---

    public int getPid() {
        return pid;
    }

    /**
     * Subscribe to stdout/stderr output from the process.
     * Returns when the process exits or the stream ends.
     */
    public void pumpOutput(OutputStream stdout, OutputStream stderr) throws IOException {
        CountDownLatch latch = new CountDownLatch(1);

        client.getExecStub().subscribeOutput(
            ExecOutputRequest.newBuilder().setPid(pid).build(),
            new StreamObserver<ExecOutputChunk>() {
                @Override
                public void onNext(ExecOutputChunk chunk) {
                    try {
                        OutputStream target = chunk.getIsStderr() ? stderr : stdout;
                        if (target != null) {
                            target.write(chunk.getData().toByteArray());
                        }
                    } catch (IOException e) {
                        // Stream consumer failed
                    }
                }

                @Override
                public void onError(Throwable t) {
                    latch.countDown();
                }

                @Override
                public void onCompleted() {
                    latch.countDown();
                }
            }
        );

        try {
            latch.await(60, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    private ExecResult createResult() {
        return new SimpleExecResult(exitCode);
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
        public ExecResult assertNormalExitValue() throws org.gradle.process.ProcessExecutionException {
            if (exitValue != 0) {
                throw new org.gradle.process.ProcessExecutionException(
                    "Process finished with non-zero exit value " + exitValue);
            }
            return this;
        }

        @Override
        public ExecResult rethrowFailure() throws org.gradle.process.ProcessExecutionException {
            return this;
        }
    }
}
