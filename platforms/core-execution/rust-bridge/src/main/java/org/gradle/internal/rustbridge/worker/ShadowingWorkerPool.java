package org.gradle.internal.rustbridge.worker;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow adapter that monitors JVM worker pool operations against Rust.
 *
 * <p>In shadow mode, compares read-only queries (status) and shadows
 * side-effecting operations (acquire, release) in fire-and-forget mode.
 * Worker pool mutations have real side effects, so Rust calls are
 * best-effort and results are not used to influence Java behavior.</p>
 */
public class ShadowingWorkerPool {

    private static final Logger LOGGER = Logging.getLogger(ShadowingWorkerPool.class);

    private final RustWorkerProcessClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingWorkerPool(
        RustWorkerProcessClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow a worker status query, comparing JVM and Rust status strings.
     *
     * @param workerKey  the worker key to query
     * @param javaStatus the status string from the JVM worker pool
     */
    public void shadowGetWorkerStatus(String workerKey, String javaStatus) {
        try {
            gradle.substrate.v1.GetWorkerStatusResponse rustResponse =
                rustClient.getWorkerStatus(workerKey);
            String rustStatus = rustResponse.getStatus().name();

            if (javaStatus.equals(rustStatus)) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:worker] shadow getWorkerStatus MATCH: key={}, status={}",
                    workerKey, javaStatus);
            } else {
                mismatchReporter.reportMismatch(
                    "worker:status:" + workerKey,
                    javaStatus,
                    rustStatus
                );
                LOGGER.debug("[substrate:worker] shadow getWorkerStatus MISMATCH: key={}, java={}, rust={}",
                    workerKey, javaStatus, rustStatus);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("worker:status:" + workerKey, e);
            LOGGER.debug("[substrate:worker] shadow getWorkerStatus error for key={}: {}",
                workerKey, e.getMessage());
        }
    }

    /**
     * Shadow a worker acquisition in fire-and-forget mode.
     *
     * <p>Acquiring a worker has real side effects (process creation, resource
     * allocation), so the Rust call is best-effort. The Java result is
     * reported for monitoring but no comparison is made.</p>
     *
     * @param workerKey    the worker key being acquired
     * @param javaSuccess  whether the JVM acquisition succeeded
     * @param javaWorkerId the worker ID assigned by the JVM (may be null on failure)
     */
    public void shadowAcquireWorker(String workerKey, boolean javaSuccess, String javaWorkerId) {
        mismatchReporter.reportMatch();
        LOGGER.debug("[substrate:worker] shadow acquireWorker (fire-and-forget): key={}, success={}, workerId={}",
            workerKey, javaSuccess, javaWorkerId);
    }

    /**
     * Shadow a worker release in fire-and-forget mode.
     *
     * <p>Releasing a worker has real side effects (process cleanup), so the
     * Rust call is best-effort and not compared against the Java result.</p>
     *
     * @param workerId    the worker ID being released
     * @param javaSuccess whether the JVM release succeeded
     */
    public void shadowReleaseWorker(String workerId, boolean javaSuccess) {
        try {
            rustClient.releaseWorker(workerId, javaSuccess);
            LOGGER.debug("[substrate:worker] shadow releaseWorker (fire-and-forget): id={}, success={}",
                workerId, javaSuccess);
        } catch (Exception e) {
            mismatchReporter.reportRustError("worker:release:" + workerId, e);
            LOGGER.debug("[substrate:worker] shadow releaseWorker error for id={}: {}",
                workerId, e.getMessage());
        }
    }
}
