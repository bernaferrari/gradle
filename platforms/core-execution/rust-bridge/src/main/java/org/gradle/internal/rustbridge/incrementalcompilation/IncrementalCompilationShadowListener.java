package org.gradle.internal.rustbridge.incrementalcompilation;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.incremental.RustIncrementalCompilationClient;
import org.slf4j.Logger;

import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Shadow listener that reports incremental compilation events to the Rust substrate's
 * {@link RustIncrementalCompilationClient}. Fire-and-forget: never affects build correctness.
 *
 * <p>Captures three categories of events:</p>
 * <ul>
 *   <li><b>Source set registration</b> -- when a source set is configured for compilation</li>
 *   <li><b>Compilation unit recording</b> -- after a single .java file is compiled</li>
 *   <li><b>Changed file reporting</b> -- before recompilation, so Rust can compute a rebuild set</li>
 * </ul>
 */
public class IncrementalCompilationShadowListener {

    private static final Logger LOGGER = Logging.getLogger(IncrementalCompilationShadowListener.class);

    private final RustIncrementalCompilationClient client;

    // -- stats counters --
    private final AtomicInteger sourceSetsRegistered = new AtomicInteger(0);
    private final AtomicInteger compilationUnitsRecorded = new AtomicInteger(0);
    private final AtomicInteger rebuildSetQueries = new AtomicInteger(0);
    private final AtomicInteger incrementalStateQueries = new AtomicInteger(0);
    private final AtomicLong totalCompileTimeMs = new AtomicLong(0);

    public IncrementalCompilationShadowListener(RustIncrementalCompilationClient client) {
        this.client = client;
    }

    // -----------------------------------------------------------------------
    // Source set registration
    // -----------------------------------------------------------------------

    /**
     * Registers a source set with the Rust incremental compilation service.
     * Called during compilation configuration (e.g. when JavaCompile task is configured).
     *
     * <p>Fire-and-forget: failures are logged at DEBUG level and silently ignored.</p>
     *
     * @param buildId       the current build identifier
     * @param sourceSetId   unique identifier for the source set
     * @param name          human-readable name (e.g. "main", "test")
     * @param sourceDirs    source directories
     * @param outputDirs    output directories
     * @param classpathHash fingerprint of the compilation classpath
     */
    public void registerSourceSet(String buildId, String sourceSetId, String name,
                                   List<String> sourceDirs, List<String> outputDirs,
                                   String classpathHash) {
        try {
            boolean accepted = client.registerSourceSet(buildId, sourceSetId, name,
                sourceDirs, outputDirs, classpathHash);
            if (accepted) {
                sourceSetsRegistered.incrementAndGet();
                LOGGER.debug("[substrate:incremental-shadow] registered source set '{}' (name={}, {} sources, {} outputs)",
                    sourceSetId, name, sourceDirs.size(), outputDirs.size());
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental-shadow] registerSourceSet failed for {}", sourceSetId, e);
        }
    }

    // -----------------------------------------------------------------------
    // Compilation unit recording
    // -----------------------------------------------------------------------

    /**
     * Records a single compilation unit (one .java file compiled to one .class file).
     * Called after each individual compilation succeeds.
     *
     * <p>Fire-and-forget: failures are logged at DEBUG level and silently ignored.</p>
     *
     * @param buildId            the current build identifier
     * @param sourceSetId        the source set this compilation belongs to
     * @param sourceFile         the compiled source file path
     * @param outputClass        the produced class file path
     * @param sourceHash         hash of the source content
     * @param classHash          hash of the produced class content
     * @param dependencies       transitive source dependencies of this unit
     * @param compileDurationMs  how long the compilation took
     */
    public void recordCompilation(String buildId, String sourceSetId, String sourceFile,
                                   String outputClass, String sourceHash, String classHash,
                                   List<String> dependencies, long compileDurationMs) {
        try {
            boolean accepted = client.recordCompilation(buildId, sourceSetId, sourceFile,
                outputClass, sourceHash, classHash, dependencies, compileDurationMs);
            if (accepted) {
                compilationUnitsRecorded.incrementAndGet();
                totalCompileTimeMs.addAndGet(compileDurationMs);
                LOGGER.debug("[substrate:incremental-shadow] recorded compilation unit: {} -> {} ({}ms, {} deps)",
                    sourceFile, outputClass, compileDurationMs, dependencies.size());
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental-shadow] recordCompilation failed for {}", sourceFile, e);
        }
    }

    // -----------------------------------------------------------------------
    // Changed file / rebuild set
    // -----------------------------------------------------------------------

    /**
     * Reports changed files to the Rust service and queries the resulting rebuild set.
     * Called before recompilation begins so the Rust side can compute which sources
     * must be recompiled.
     *
     * <p>Fire-and-forget: failures are logged at DEBUG level and silently ignored.
     * The caller should not use the result to drive build decisions.</p>
     *
     * @param buildId       the current build identifier
     * @param sourceSetId   the source set to query
     * @param changedFiles  files detected as changed since the last compilation
     */
    public void reportChangedFiles(String buildId, String sourceSetId, List<String> changedFiles) {
        try {
            rebuildSetQueries.incrementAndGet();
            gradle.substrate.v1.GetRebuildSetResponse response = client.getRebuildSet(buildId, sourceSetId, changedFiles);

            LOGGER.debug("[substrate:incremental-shadow] rebuild set for '{}': {} total sources, {} must recompile, {} up-to-date",
                sourceSetId, response.getTotalSources(), response.getMustRecompileCount(), response.getUpToDateCount());

            if (response.getDecisionsCount() > 0) {
                for (gradle.substrate.v1.RebuildDecision decision : response.getDecisionsList()) {
                    LOGGER.debug("[substrate:incremental-shadow]   {}: must_recompile={}, reason={}",
                        decision.getSourceFile(), decision.getMustRecompile(), decision.getReason());
                }
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental-shadow] reportChangedFiles failed for source set {}", sourceSetId, e);
        }
    }

    // -----------------------------------------------------------------------
    // Incremental state query
    // -----------------------------------------------------------------------

    /**
     * Queries the current incremental compilation state from Rust.
     * Useful for diagnostics and logging.
     *
     * <p>Fire-and-forget: failures are logged at DEBUG level and silently ignored.</p>
     *
     * @param buildId     the current build identifier
     * @param sourceSetId the source set to query
     */
    public void queryIncrementalState(String buildId, String sourceSetId) {
        try {
            incrementalStateQueries.incrementAndGet();
            gradle.substrate.v1.GetIncrementalStateResponse response = client.getIncrementalState(buildId, sourceSetId);
            gradle.substrate.v1.IncrementalState state = response.getState();

            LOGGER.debug("[substrate:incremental-shadow] incremental state for '{}': total={}, incremental={}, full={}, compile_time={}ms, classpath_changed={}",
                sourceSetId, state.getTotalCompiled(), state.getIncrementallyCompiled(),
                state.getFullyRecompiled(), state.getTotalCompileTimeMs(),
                state.getClasspathChanged());
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental-shadow] queryIncrementalState failed for source set {}", sourceSetId, e);
        }
    }

    // -----------------------------------------------------------------------
    // Stats getters
    // -----------------------------------------------------------------------

    /**
     * Returns the number of source sets that were successfully registered with Rust.
     */
    public int getSourceSetsRegistered() {
        return sourceSetsRegistered.get();
    }

    /**
     * Returns the number of compilation units that were successfully recorded with Rust.
     */
    public int getCompilationUnitsRecorded() {
        return compilationUnitsRecorded.get();
    }

    /**
     * Returns the number of rebuild set queries issued to Rust.
     */
    public int getRebuildSetQueries() {
        return rebuildSetQueries.get();
    }

    /**
     * Returns the number of incremental state queries issued to Rust.
     */
    public int getIncrementalStateQueries() {
        return incrementalStateQueries.get();
    }

    /**
     * Returns the total compilation time (in ms) across all recorded compilation units.
     */
    public long getTotalCompileTimeMs() {
        return totalCompileTimeMs.get();
    }
}
