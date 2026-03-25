package org.gradle.internal.rustbridge.eventstream;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Collections;

/**
 * Shadow adapter that compares JVM build event logging with Rust.
 *
 * <p>In shadow mode, sends build events and queries event logs
 * through both JVM and Rust paths, reporting mismatches.</p>
 */
public class ShadowingBuildEventLogger {

    private static final Logger LOGGER = Logging.getLogger(ShadowingBuildEventLogger.class);

    private final RustBuildEventStreamClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingBuildEventLogger(
        RustBuildEventStreamClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Fire-and-forget shadow of a build event send.
     *
     * <p>Sends the event to Rust, catches any errors, and reports
     * via the mismatch reporter. Does not compare results since
     * event sends are side-effect-only operations.</p>
     *
     * @param buildId    the build identifier
     * @param eventType  the event type string
     * @param eventId    the event identifier
     * @param javaResult whether the JVM event send succeeded
     */
    public void shadowSendEvent(String buildId, String eventType, String eventId, boolean javaResult) {
        try {
            boolean rustResult = rustClient.sendBuildEvent(
                buildId, eventType, eventId, Collections.emptyMap(), "", ""
            );

            if (javaResult == rustResult) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:event-stream] shadow sendEvent MATCH: buildId={}, type={}, id={}",
                    buildId, eventType, eventId);
            } else {
                mismatchReporter.reportMismatch(
                    "event-stream:sendEvent:" + buildId + ":" + eventId,
                    String.valueOf(javaResult),
                    String.valueOf(rustResult)
                );
                LOGGER.debug("[substrate:event-stream] shadow sendEvent MISMATCH: buildId={}, type={}, id={}, java={}, rust={}",
                    buildId, eventType, eventId, javaResult, rustResult);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("event-stream:sendEvent:" + buildId + ":" + eventId, e);
            LOGGER.debug("[substrate:event-stream] shadow sendEvent error for buildId={}, id={}: {}",
                buildId, eventId, e.getMessage());
        }
    }

    /**
     * Shadow a get-event-log query, comparing event counts.
     *
     * <p>Queries the Rust event log and compares the event count
     * with the Java event count.</p>
     *
     * @param buildId        the build identifier
     * @param javaEventCount the number of events in the JVM log
     */
    public void shadowGetEventLog(String buildId, int javaEventCount) {
        try {
            gradle.substrate.v1.GetEventLogResponse rustResponse =
                rustClient.getEventLog(buildId, 0, 1000);
            int rustEventCount = rustResponse.getEventsCount();

            if (javaEventCount == rustEventCount) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:event-stream] shadow getEventLog MATCH: buildId={}, count={}",
                    buildId, javaEventCount);
            } else {
                mismatchReporter.reportMismatch(
                    "event-stream:getEventLog:" + buildId,
                    String.valueOf(javaEventCount),
                    String.valueOf(rustEventCount)
                );
                LOGGER.debug("[substrate:event-stream] shadow getEventLog MISMATCH: buildId={}, java={}, rust={}",
                    buildId, javaEventCount, rustEventCount);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("event-stream:getEventLog:" + buildId, e);
            LOGGER.debug("[substrate:event-stream] shadow getEventLog error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }
}
