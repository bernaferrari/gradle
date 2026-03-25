package org.gradle.internal.rustbridge.worker;

import gradle.substrate.v1.AcquireWorkerRequest;
import gradle.substrate.v1.AcquireWorkerResponse;
import gradle.substrate.v1.ConfigurePoolRequest;
import gradle.substrate.v1.ConfigurePoolResponse;
import gradle.substrate.v1.GetWorkerStatusRequest;
import gradle.substrate.v1.GetWorkerStatusResponse;
import gradle.substrate.v1.ReleaseWorkerRequest;
import gradle.substrate.v1.ReleaseWorkerResponse;
import gradle.substrate.v1.StopWorkerRequest;
import gradle.substrate.v1.StopWorkerResponse;
import gradle.substrate.v1.WorkerProcessServiceGrpc;
import gradle.substrate.v1.WorkerSpec;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;
import java.util.Map;

/**
 * Client for the Rust worker process service.
 * Acquires, releases, and manages worker processes via gRPC.
 */
public class RustWorkerProcessClient {

    private static final Logger LOGGER = Logging.getLogger(RustWorkerProcessClient.class);

    private final SubstrateClient client;

    public RustWorkerProcessClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Descriptor for an acquired worker process.
     */
    public static class WorkerHandle {
        private final String workerId;
        private final String workerKey;
        private final int pid;
        private final String connectAddress;
        private final long startedAtMs;
        private final boolean healthy;

        private WorkerHandle(String workerId, String workerKey, int pid,
                             String connectAddress, long startedAtMs, boolean healthy) {
            this.workerId = workerId;
            this.workerKey = workerKey;
            this.pid = pid;
            this.connectAddress = connectAddress;
            this.startedAtMs = startedAtMs;
            this.healthy = healthy;
        }

        public String getWorkerId() { return workerId; }
        public String getWorkerKey() { return workerKey; }
        public int getPid() { return pid; }
        public String getConnectAddress() { return connectAddress; }
        public long getStartedAtMs() { return startedAtMs; }
        public boolean isHealthy() { return healthy; }
    }

    /**
     * Result of acquiring a worker.
     */
    public static class AcquireResult {
        private final boolean success;
        private final WorkerHandle worker;
        private final boolean reused;
        private final String errorMessage;

        private AcquireResult(boolean success, WorkerHandle worker, boolean reused, String errorMessage) {
            this.success = success;
            this.worker = worker;
            this.reused = reused;
            this.errorMessage = errorMessage;
        }

        public boolean isSuccess() { return success; }
        public WorkerHandle getWorker() { return worker; }
        public boolean isReused() { return reused; }
        public String getErrorMessage() { return errorMessage; }
    }

    /**
     * Acquire a worker process from the pool.
     */
    public AcquireResult acquireWorker(String workerKey, String javaHome, List<String> classpath,
                                        String workingDir, Map<String, String> jvmArgs,
                                        int maxMemoryMb, boolean daemon, long timeoutMs) {
        if (client.isNoop()) {
            return new AcquireResult(false, null, false, "Substrate not available");
        }

        try {
            WorkerSpec spec = WorkerSpec.newBuilder()
                .setWorkerKey(workerKey)
                .setJavaHome(javaHome != null ? javaHome : "")
                .addAllClasspath(classpath)
                .setWorkingDir(workingDir)
                .putAllJvmArgs(jvmArgs)
                .setMaxMemoryMb(maxMemoryMb)
                .setDaemon(daemon)
                .build();

            AcquireWorkerResponse response = client.getWorkerProcessStub()
                .acquireWorker(AcquireWorkerRequest.newBuilder()
                    .setSpec(spec)
                    .setTimeoutMs(timeoutMs)
                    .build());

            if (response.hasWorker()) {
                gradle.substrate.v1.WorkerHandle w = response.getWorker();
                WorkerHandle handle = new WorkerHandle(
                    w.getWorkerId(), w.getWorkerKey(), w.getPid(),
                    w.getConnectAddress(), w.getStartedAtMs(), w.getHealthy());
                LOGGER.debug("[substrate:worker] acquired worker {} (pid={}, reused={})",
                    w.getWorkerId(), w.getPid(), response.getReused());
                return new AcquireResult(true, handle, response.getReused(), null);
            } else {
                LOGGER.debug("[substrate:worker] acquire failed: {}", response.getErrorMessage());
                return new AcquireResult(false, null, false, response.getErrorMessage());
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:worker] acquire worker failed", e);
            return new AcquireResult(false, null, false, e.getMessage());
        }
    }

    /**
     * Release a worker back to the pool.
     */
    public boolean releaseWorker(String workerId, boolean healthy) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ReleaseWorkerResponse response = client.getWorkerProcessStub()
                .releaseWorker(ReleaseWorkerRequest.newBuilder()
                    .setWorkerId(workerId)
                    .setHealthy(healthy)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:worker] release worker failed", e);
            return false;
        }
    }

    /**
     * Stop a specific worker process.
     */
    public boolean stopWorker(String workerId, boolean force) {
        if (client.isNoop()) {
            return false;
        }

        try {
            StopWorkerResponse response = client.getWorkerProcessStub()
                .stopWorker(StopWorkerRequest.newBuilder()
                    .setWorkerId(workerId)
                    .setForce(force)
                    .build());
            return response.getStopped();
        } catch (Exception e) {
            LOGGER.debug("[substrate:worker] stop worker failed", e);
            return false;
        }
    }

    /**
     * Get status of all worker processes.
     */
    public GetWorkerStatusResponse getWorkerStatus(String workerKey) {
        if (client.isNoop()) {
            return GetWorkerStatusResponse.getDefaultInstance();
        }

        try {
            GetWorkerStatusRequest.Builder builder = GetWorkerStatusRequest.newBuilder();
            if (workerKey != null) {
                builder.setWorkerKey(workerKey);
            }
            return client.getWorkerProcessStub()
                .getWorkerStatus(builder.build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:worker] get worker status failed", e);
            return GetWorkerStatusResponse.getDefaultInstance();
        }
    }

    /**
     * Configure the worker pool.
     */
    public boolean configurePool(int maxPoolSize, long idleTimeoutMs, int maxPerKey,
                                  boolean enableHealthChecks) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ConfigurePoolResponse response = client.getWorkerProcessStub()
                .configurePool(ConfigurePoolRequest.newBuilder()
                    .setMaxPoolSize(maxPoolSize)
                    .setIdleTimeoutMs(idleTimeoutMs)
                    .setMaxPerKey(maxPerKey)
                    .setEnableHealthChecks(enableHealthChecks)
                    .build());
            return response.getApplied();
        } catch (Exception e) {
            LOGGER.debug("[substrate:worker] configure pool failed", e);
            return false;
        }
    }
}
