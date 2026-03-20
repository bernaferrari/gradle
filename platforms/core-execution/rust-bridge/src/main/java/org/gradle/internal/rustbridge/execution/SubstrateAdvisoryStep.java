package org.gradle.internal.rustbridge.execution;

import com.google.common.collect.ImmutableList;
import com.google.common.collect.ImmutableSortedMap;
import org.gradle.api.logging.Logging;
import org.gradle.internal.execution.Execution;
import org.gradle.internal.execution.UnitOfWork;
import org.gradle.internal.execution.history.ExecutionOutputState;
import org.gradle.internal.execution.history.PreviousExecutionState;
import org.gradle.internal.execution.history.impl.DefaultExecutionOutputState;
import org.gradle.internal.execution.steps.AfterExecutionResult;
import org.gradle.internal.execution.steps.MutableChangesContext;
import org.gradle.internal.execution.steps.Step;
import org.gradle.internal.execution.steps.UpToDateResult;
import org.gradle.internal.fingerprint.CurrentFileCollectionFingerprint;
import org.gradle.internal.hash.Hasher;
import org.gradle.internal.hash.Hashing;
import org.gradle.internal.snapshot.ValueSnapshot;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Execution pipeline step that consults the Rust substrate for execution planning.
 *
 * Phase 5 (advisory): Predicts the outcome via Rust, logs comparison with Java's decision.
 * Phase 6 (authoritative): Rust's plan resolution drives the actual execution decision.
 *
 * This step wraps the SkipUpToDateStep in the mutable pipeline, intercepting
 * the decision point between skip/execute.
 */
public class SubstrateAdvisoryStep<C extends MutableChangesContext> implements Step<C, UpToDateResult> {

    private static final Logger LOGGER = Logging.getLogger(SubstrateAdvisoryStep.class);

    private final Step<? super C, ? extends AfterExecutionResult> delegate;
    private final ExecutionPlanClient planClient;
    private final boolean authoritative;

    public SubstrateAdvisoryStep(
        Step<? super C, ? extends AfterExecutionResult> delegate,
        ExecutionPlanClient planClient,
        boolean authoritative
    ) {
        this.delegate = delegate;
        this.planClient = planClient;
        this.authoritative = authoritative;
    }

    @Override
    public UpToDateResult execute(UnitOfWork work, C context) {
        if (planClient == null) {
            return wrapResult(work, context, delegate.execute(work, context));
        }

        // Build work metadata from the execution context
        gradle.substrate.v1.WorkMetadata workMetadata = buildWorkMetadata(work, context);

        // Phase 5: Advisory - predict and compare with Java's decision
        ExecutionPlanClient.Prediction prediction = planClient.predictOutcome(workMetadata);

        ImmutableList<String> rebuildReasons = context.getRebuildReasons();
        boolean javaWantsToExecute = !rebuildReasons.isEmpty() || !context.getChanges().isPresent();

        // Log the comparison
        logComparison(work, prediction, javaWantsToExecute, rebuildReasons);

        // Phase 6: Authoritative mode - use Rust's plan resolution
        if (authoritative) {
            return executeAuthoritative(work, context, workMetadata);
        }

        // Phase 5: Advisory mode - Java remains authoritative, just record for comparison
        return executeAdvisory(work, context, prediction);
    }

    private UpToDateResult wrapResult(UnitOfWork work, C context, AfterExecutionResult result) {
        // If the delegate already returned UpToDateResult, pass through
        if (result instanceof UpToDateResult) {
            return (UpToDateResult) result;
        }
        // Otherwise, wrap it like SkipUpToDateStep.executeBecause does
        return new UpToDateResult(result, ImmutableList.of());
    }

    private UpToDateResult executeAdvisory(
        UnitOfWork work,
        C context,
        ExecutionPlanClient.Prediction prediction
    ) {
        // Let Java make the decision
        long startTime = System.currentTimeMillis();
        UpToDateResult javaResult = wrapResult(work, context, delegate.execute(work, context));
        long duration = System.currentTimeMillis() - startTime;

        // Record the actual outcome for shadow comparison
        String actualOutcome = javaResult.getExecution()
            .map(Execution::getOutcome)
            .map(Object::toString)
            .orElse("NO_EXECUTION");

        boolean predictionCorrect = isPredictionCorrect(prediction, actualOutcome);

        if (LOGGER.isDebugEnabled()) {
            LOGGER.debug("[substrate:advisory] {} predicted={} actual={} correct={} duration={}ms",
                work.getDisplayName(), prediction, actualOutcome, predictionCorrect, duration);
        }

        String workIdentity = context.getIdentity().getUniqueId();
        planClient.recordOutcome(workIdentity, prediction, actualOutcome, predictionCorrect, duration);

        return javaResult;
    }

    private UpToDateResult executeAuthoritative(
        UnitOfWork work,
        C context,
        gradle.substrate.v1.WorkMetadata workMetadata
    ) {
        ExecutionPlanClient.PlanResolution plan = planClient.resolvePlan(workMetadata, true);

        if (plan.shouldSkip()) {
            return skipExecution(work, context, plan.getReasoning());
        }

        // For FROM_CACHE and SHORT_CIRCUIT, we still let Java handle the actual
        // cache loading/short-circuiting since it needs to wire up the execution context.
        // Rust's decision is used to validate or override the skip decision only.
        // Full authoritative cache handling comes when the cache service is fully
        // migrated in later phases.

        // Fall through to Java's execution with Rust's plan recorded
        long startTime = System.currentTimeMillis();
        UpToDateResult result = wrapResult(work, context, delegate.execute(work, context));
        long duration = System.currentTimeMillis() - startTime;

        String actualOutcome = result.getExecution()
            .map(Execution::getOutcome)
            .map(Object::toString)
            .orElse("NO_EXECUTION");

        LOGGER.info("[substrate:authoritative] {} plan_action={} actual={}",
            work.getDisplayName(), plan.getAction(), actualOutcome);

        return result;
    }

    private UpToDateResult skipExecution(UnitOfWork work, C context, String reason) {
        LOGGER.lifecycle("[substrate:authoritative] Skipping {} - {}", work.getDisplayName(), reason);

        Optional<PreviousExecutionState> prevState = context.getPreviousExecutionState();
        if (!prevState.isPresent()) {
            // No previous state, must execute
            return wrapResult(work, context, delegate.execute(work, context));
        }

        PreviousExecutionState previousExecutionState = prevState.get();
        ExecutionOutputState executionOutputState = new DefaultExecutionOutputState(
            true,
            previousExecutionState.getOutputFilesProducedByWork(),
            previousExecutionState.getOriginMetadata(),
            true
        );

        org.gradle.internal.Try<Execution> execution = org.gradle.internal.Try.successful(
            Execution.skipped(Execution.ExecutionOutcome.UP_TO_DATE, work)
        );

        return new UpToDateResult(
            previousExecutionState.getOriginMetadata().getExecutionTime(),
            execution,
            executionOutputState,
            ImmutableList.of(reason),
            previousExecutionState.getOriginMetadata()
        );
    }

    private gradle.substrate.v1.WorkMetadata buildWorkMetadata(UnitOfWork work, C context) {
        ImmutableSortedMap<String, ValueSnapshot> inputProps = context.getInputProperties();
        ImmutableSortedMap<String, CurrentFileCollectionFingerprint> inputFileProps = context.getInputFileProperties();

        Map<String, String> propsMap = new HashMap<>();
        for (Map.Entry<String, ValueSnapshot> entry : inputProps.entrySet()) {
            Hasher hasher = Hashing.md5().newHasher();
            entry.getValue().appendToHasher(hasher);
            propsMap.put(entry.getKey(), hasher.hash().toString());
        }

        Map<String, String> fileFpMap = new HashMap<>();
        for (Map.Entry<String, CurrentFileCollectionFingerprint> entry : inputFileProps.entrySet()) {
            fileFpMap.put(entry.getKey(), entry.getValue().getHash().toString());
        }

        boolean cachingEnabled = false;
        if (context instanceof org.gradle.internal.execution.steps.CachingContext) {
            org.gradle.internal.execution.caching.CachingState cachingState =
                ((org.gradle.internal.execution.steps.CachingContext) context).getCachingState();
            cachingEnabled = cachingState.fold(
                enabled -> true,
                disabled -> false
            );
        }

        return planClient.buildWorkMetadata(
            context.getIdentity().getUniqueId(),
            work.getDisplayName(),
            context.getImplementation().getClassIdentifier(),
            propsMap,
            fileFpMap,
            cachingEnabled,
            work.isAllowedToLoadFromCache(),
            context.getPreviousExecutionState().isPresent(),
            context.getRebuildReasons()
        );
    }

    private boolean isPredictionCorrect(ExecutionPlanClient.Prediction prediction, String actualOutcome) {
        switch (prediction) {
            case EXECUTE:
                return !actualOutcome.equals("UP_TO_DATE")
                    && !actualOutcome.equals("FROM_CACHE")
                    && !actualOutcome.equals("SHORT_CIRCUITED");
            case UP_TO_DATE:
                return actualOutcome.equals("UP_TO_DATE");
            case FROM_CACHE:
                return actualOutcome.equals("FROM_CACHE");
            case SHORT_CIRCUITED:
                return actualOutcome.equals("SHORT_CIRCUITED");
            default:
                return true; // UNKNOWN is never wrong
        }
    }

    private void logComparison(
        UnitOfWork work,
        ExecutionPlanClient.Prediction prediction,
        boolean javaWantsToExecute,
        List<String> rebuildReasons
    ) {
        if (!LOGGER.isDebugEnabled()) {
            return;
        }
        String javaDecision = javaWantsToExecute
            ? "EXECUTE (reasons: " + rebuildReasons + ")"
            : "SKIP";
        LOGGER.debug("[substrate:advisory] {} rust_prediction={} java_decision={}",
            work.getDisplayName(), prediction, javaDecision);
    }
}
