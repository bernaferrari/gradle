package org.gradle.internal.rustbridge.exec;

import gradle.substrate.v1.ExecSpawnRequest;
import gradle.substrate.v1.ExecSpawnResponse;
import gradle.substrate.v1.ExecWaitRequest;
import gradle.substrate.v1.ExecWaitResponse;
import gradle.substrate.v1.ExecOutputChunk;
import gradle.substrate.v1.ExecOutputRequest;
import io.grpc.stub.StreamObserver;
import org.gradle.internal.rustbridge.SubstrateClient;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/**
 * Process handle backed by the Rust substrate daemon.
 * Delegates spawn/wait/signal/kill to the Rust ExecService via gRPC.
 */
public class RustProcessHandle implements org.gradle.process.internal.ExecHandle {

    private final SubstrateClient client;
    private final int pid;
    private volatile boolean alive = true;

    public RustProcessHandle(SubstrateClient client, int pid) {
        this.client = client;
        this.pid = pid;
    }

    @Override
    public int waitFor() throws InterruptedException {
        if (!alive) {
            return exitCode;
        }

        ExecWaitResponse response = client.getExecStub().waitExec(
            ExecWaitRequest.newBuilder().setPid(pid).build()
        );

        alive = false;
        exitCode = response.getExitCode();
        return exitCode;
    }

    private int exitCode;

    @Override
    public boolean isAlive() {
        return alive;
    }

    @Override
    public void abort() {
        client.getExecStub().killTree(
            gradle.substrate.v1.ExecKillTreeRequest.newBuilder().setPid(pid).build()
        );
        alive = false;
    }

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
}
