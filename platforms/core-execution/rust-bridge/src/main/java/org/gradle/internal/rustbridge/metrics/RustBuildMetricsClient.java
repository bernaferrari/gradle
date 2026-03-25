package org.gradle.internal.rustbridge.metrics;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust build metrics service.
 * Records and retrieves build performance metrics via gRPC.
 */
public class RustBuildMetricsClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildMetricsClient.class);

    private final SubstrateClient client;

    public RustBuildMetricsClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Record a counter metric event.
     */
    public void recordCounter(String buildId, String name, double value) {
        recordMetric(buildId, name, String.valueOf(value), "counter", new HashMap<>());
    }

    /**
     * Record a counter metric with tags.
     */
    public void recordCounter(String buildId, String name, double value, Map<String, String> tags) {
        recordMetric(buildId, name, String.valueOf(value), "counter", tags);
    }

    /**
     * Record a timer metric.
     */
    public void recordTimer(String buildId, String name, long durationMs) {
        recordMetric(buildId, name, String.valueOf(durationMs), "timer", new HashMap<>());
    }

    /**
     * Record a gauge metric.
     */
    public void recordGauge(String buildId, String name, double value) {
        recordMetric(buildId, name, String.valueOf(value), "gauge", new HashMap<>());
    }

    private void recordMetric(String buildId, String name, String value, String type, Map<String, String> tags) {
        if (client.isNoop()) {
            return;
        }

        try {
            gradle.substrate.v1.MetricEvent event = gradle.substrate.v1.MetricEvent.newBuilder()
                .setName(name)
                .setValue(value)
                .setMetricType(type)
                .putAllTags(tags)
                .setTimestampMs(System.currentTimeMillis())
                .build();

            gradle.substrate.v1.RecordMetricRequest request = gradle.substrate.v1.RecordMetricRequest.newBuilder()
                .setBuildId(buildId != null ? buildId : "")
                .setEvent(event)
                .build();

            client.getBuildMetricsStub().recordMetric(request);
        } catch (Exception e) {
            LOGGER.debug("[substrate:metrics] failed to record {}: {}", name, e.getMessage());
        }
    }

    /**
     * A performance summary from the Rust metrics service.
     */
    public static class PerformanceSummary {
        private final String buildId;
        private final long durationMs;
        private final int totalTasks;
        private final int cachedTasks;
        private final int upToDateTasks;
        private final int executedTasks;
        private final int failedTasks;
        private final double cacheHitRate;
        private final long bytesStored;
        private final long bytesLoaded;
        private final String outcome;

        private PerformanceSummary(gradle.substrate.v1.PerformanceSummary proto) {
            this.buildId = proto.getBuildId();
            this.durationMs = proto.getDurationMs();
            this.totalTasks = proto.getTotalTasksExecuted();
            this.cachedTasks = proto.getTasksFromCache();
            this.upToDateTasks = proto.getTasksUpToDate();
            this.executedTasks = proto.getTasksExecuted();
            this.failedTasks = proto.getTasksFailed();
            this.cacheHitRate = proto.getBuildCacheHitRate();
            this.bytesStored = proto.getTotalBytesStored();
            this.bytesLoaded = proto.getTotalBytesLoaded();
            this.outcome = proto.getBuildOutcome();
        }

        public String getBuildId() { return buildId; }
        public long getDurationMs() { return durationMs; }
        public int getTotalTasks() { return totalTasks; }
        public int getCachedTasks() { return cachedTasks; }
        public int getUpToDateTasks() { return upToDateTasks; }
        public int getExecutedTasks() { return executedTasks; }
        public int getFailedTasks() { return failedTasks; }
        public double getCacheHitRate() { return cacheHitRate; }
        public long getBytesStored() { return bytesStored; }
        public long getBytesLoaded() { return bytesLoaded; }
        public String getOutcome() { return outcome; }

        @Override
        public String toString() {
            return String.format("PerformanceSummary{build=%s, duration=%dms, tasks=%d, " +
                "cached=%d, executed=%d, failed=%d, cacheHitRate=%.1f%%, outcome=%s}",
                buildId, durationMs, totalTasks, cachedTasks, executedTasks,
                failedTasks, cacheHitRate * 100, outcome);
        }
    }

    /**
     * Get aggregated metrics for a build.
     */
    public List<gradle.substrate.v1.MetricSnapshot> getMetrics(String buildId,
                                                               List<String> metricNames,
                                                               long sinceMs) {
        if (client.isNoop()) {
            return java.util.Collections.emptyList();
        }

        try {
            gradle.substrate.v1.GetMetricsRequest.Builder request =
                gradle.substrate.v1.GetMetricsRequest.newBuilder()
                    .setBuildId(buildId != null ? buildId : "")
                    .setSinceMs(sinceMs);

            if (metricNames != null) {
                request.addAllMetricNames(metricNames);
            }

            return client.getBuildMetricsStub()
                .getMetrics(request.build())
                .getMetricsList();
        } catch (Exception e) {
            LOGGER.debug("[substrate:metrics] failed to get metrics: {}", e.getMessage());
            return java.util.Collections.emptyList();
        }
    }

    /**
     * Get the performance summary for a build.
     */
    public PerformanceSummary getPerformanceSummary(String buildId) {
        if (client.isNoop()) {
            return null;
        }

        try {
            gradle.substrate.v1.GetPerformanceSummaryRequest request =
                gradle.substrate.v1.GetPerformanceSummaryRequest.newBuilder()
                    .setBuildId(buildId)
                    .build();

            gradle.substrate.v1.GetPerformanceSummaryResponse response =
                client.getBuildMetricsStub().getPerformanceSummary(request);

            if (response.hasSummary()) {
                return new PerformanceSummary(response.getSummary());
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:metrics] failed to get summary: {}", e.getMessage());
        }
        return null;
    }

    /**
     * Reset all metrics for a build.
     */
    public void resetMetrics(String buildId) {
        if (client.isNoop()) {
            return;
        }

        try {
            gradle.substrate.v1.ResetMetricsRequest request =
                gradle.substrate.v1.ResetMetricsRequest.newBuilder()
                    .setBuildId(buildId != null ? buildId : "")
                    .build();

            client.getBuildMetricsStub().resetMetrics(request);
        } catch (Exception e) {
            LOGGER.debug("[substrate:metrics] failed to reset metrics: {}", e.getMessage());
        }
    }
}
