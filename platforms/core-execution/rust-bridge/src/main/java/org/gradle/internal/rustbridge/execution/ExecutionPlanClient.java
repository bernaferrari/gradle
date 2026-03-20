package org.gradle.internal.rustbridge.execution;

import gradle.substrate.v1.PlanAction;
import gradle.substrate.v1.PredictOutcomeRequest;
import gradle.substrate.v1.PredictOutcomeResponse;
import gradle.substrate.v1.PredictedOutcome;
import gradle.substrate.v1.RecordOutcomeRequest;
import gradle.substrate.v1.ResolvePlanRequest;
import gradle.substrate.v1.ResolvePlanResponse;
import gradle.substrate.v1.WorkMetadata;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust execution plan service.
 * Provides advisory predictions and authoritative plan resolution.
 */
public class ExecutionPlanClient {

    private static final Logger LOGGER = Logging.getLogger(ExecutionPlanClient.class);

    private final SubstrateClient client;

    public ExecutionPlanClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Phase 5: Ask Rust to predict the execution outcome without side effects.
     */
    public Prediction predictOutcome(WorkMetadata work) {
        if (client.isNoop()) {
            return Prediction.UNKNOWN;
        }

        try {
            PredictOutcomeResponse response = client.getExecutionPlanStub().predictOutcome(
                PredictOutcomeRequest.newBuilder().setWork(work).build()
            );

            PredictedOutcome predicted = response.getPredictedOutcome();
            LOGGER.debug("[substrate:plan] predicted={} confidence={} reasoning='{}'",
                predicted, response.getConfidence(), response.getReasoning());

            return Prediction.fromProto(predicted);
        } catch (Exception e) {
            LOGGER.debug("[substrate:plan] prediction failed, falling back to UNKNOWN", e);
            return Prediction.UNKNOWN;
        }
    }

    /**
     * Phase 6: Ask Rust to authoritatively resolve the execution plan.
     */
    public PlanResolution resolvePlan(WorkMetadata work, boolean authoritative) {
        if (client.isNoop()) {
            return PlanResolution.execute("Substrate not available");
        }

        try {
            ResolvePlanResponse response = client.getExecutionPlanStub().resolvePlan(
                ResolvePlanRequest.newBuilder()
                    .setWork(work)
                    .setAuthoritative(authoritative)
                    .build()
            );

            PlanAction action = response.getAction();
            LOGGER.debug("[substrate:plan] action={} reasoning='{}' cache_key_hint='{}'",
                action, response.getReasoning(), response.getCacheKeyHint());

            return PlanResolution.fromProto(action, response.getReasoning(), response.getCacheKeyHint());
        } catch (Exception e) {
            LOGGER.debug("[substrate:plan] plan resolution failed, executing", e);
            return PlanResolution.execute("Substrate unavailable: " + e.getMessage());
        }
    }

    /**
     * Record the actual outcome for shadow mode comparison.
     */
    public void recordOutcome(String workIdentity, Prediction predicted,
                             String actualOutcome, boolean predictionCorrect, long durationMs) {
        if (client.isNoop()) {
            return;
        }

        try {
            client.getExecutionPlanStub().recordOutcome(
                RecordOutcomeRequest.newBuilder()
                    .setWorkIdentity(workIdentity)
                    .setPredictedOutcome(predicted.toProto())
                    .setActualOutcome(actualOutcome)
                    .setPredictionCorrect(predictionCorrect)
                    .setDurationMs(durationMs)
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:plan] failed to record outcome", e);
        }
    }

    /**
     * Build a WorkMetadata protobuf from Gradle's execution context.
     */
    public WorkMetadata buildWorkMetadata(
        String workIdentity,
        String displayName,
        String implementationClass,
        Map<String, String> inputProperties,
        Map<String, String> inputFileFingerprints,
        boolean cachingEnabled,
        boolean canLoadFromCache,
        boolean hasPreviousExecutionState,
        List<String> rebuildReasons
    ) {
        WorkMetadata.Builder builder = WorkMetadata.newBuilder()
            .setWorkIdentity(workIdentity)
            .setDisplayName(displayName)
            .setImplementationClass(implementationClass)
            .putAllInputProperties(inputProperties)
            .putAllInputFileFingerprints(inputFileFingerprints)
            .setCachingEnabled(cachingEnabled)
            .setCanLoadFromCache(canLoadFromCache)
            .setHasPreviousExecutionState(hasPreviousExecutionState)
            .addAllRebuildReasons(rebuildReasons);
        return builder.build();
    }

    public enum Prediction {
        UNKNOWN(PredictedOutcome.PREDICTED_UNKNOWN),
        EXECUTE(PredictedOutcome.PREDICTED_EXECUTE),
        UP_TO_DATE(PredictedOutcome.PREDICTED_UP_TO_DATE),
        FROM_CACHE(PredictedOutcome.PREDICTED_FROM_CACHE),
        SHORT_CIRCUITED(PredictedOutcome.PREDICTED_SHORT_CIRCUITED);

        private final PredictedOutcome proto;

        Prediction(PredictedOutcome proto) {
            this.proto = proto;
        }

        public PredictedOutcome toProto() {
            return proto;
        }

        static Prediction fromProto(PredictedOutcome proto) {
            switch (proto) {
                case PREDICTED_EXECUTE: return EXECUTE;
                case PREDICTED_UP_TO_DATE: return UP_TO_DATE;
                case PREDICTED_FROM_CACHE: return FROM_CACHE;
                case PREDICTED_SHORT_CIRCUITED: return SHORT_CIRCUITED;
                default: return UNKNOWN;
            }
        }
    }

    public static class PlanResolution {
        private final PlanAction action;
        private final String reasoning;
        private final String cacheKeyHint;

        private PlanResolution(PlanAction action, String reasoning, String cacheKeyHint) {
            this.action = action;
            this.reasoning = reasoning;
            this.cacheKeyHint = cacheKeyHint;
        }

        public PlanAction getAction() { return action; }
        public String getReasoning() { return reasoning; }
        public String getCacheKeyHint() { return cacheKeyHint; }

        public boolean shouldSkip() {
            return action == PlanAction.PLAN_ACTION_SKIP_UP_TO_DATE;
        }

        public boolean shouldLoadFromCache() {
            return action == PlanAction.PLAN_ACTION_LOAD_FROM_CACHE;
        }

        public boolean shouldShortCircuit() {
            return action == PlanAction.PLAN_ACTION_SHORT_CIRCUIT;
        }

        public boolean shouldExecute() {
            return action == PlanAction.PLAN_ACTION_EXECUTE || action == PlanAction.PLAN_ACTION_UNKNOWN;
        }

        static PlanResolution fromProto(PlanAction action, String reasoning, String cacheKeyHint) {
            return new PlanResolution(action, reasoning, cacheKeyHint);
        }

        static PlanResolution execute(String reasoning) {
            return new PlanResolution(PlanAction.PLAN_ACTION_EXECUTE, reasoning, "");
        }
    }
}
