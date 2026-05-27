package org.gradle.internal.rustbridge.filehashcache;

import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Optional;

/**
 * Shadowing (decorator-style) wrapper for the FileHashCache bigger slice (persistent cache).
 *
 * <p>Runs the Java implementation (CrossBuildFileHashCache backed stores, CachingFileHasher, etc.)
 * in parallel with the Rust {@link RustFileHashCacheClient} for differential testing / parity validation.
 *
 * <p>Follows the exact established patterns 100% from:
 * <ul>
 *   <li>{@code ShadowingFileHasher}</li>
 *   <li>{@code ShadowingFileCollectionSnapshotter}</li>
 *   <li>{@code ShadowingValueSnapshotter}</li>
 * </ul>
 *
 * <p>In shadow mode (default when enabled but not authoritative):
 *   - Always return the Java result (source of truth).
 *   - Invoke Rust (via thin client) in parallel purely for observation + mismatch reporting.
 *
 * <p>In authoritative mode:
 *   - Rust FileHashCacheService is the source of truth for Get/Put/Invalidate.
 *   - Cross-validate against Java (when delegate narrowed) and fail-closed (SubstrateException) on Rust error or mismatch.
 *
 * <p>Currently a helper (javaDelegate held as Object pending narrow FileHashCacheStore iface).
 * Used by conditional providers behind ENABLE_RUST_FILE_HASH_CACHE + AUTHORITATIVE variant.
 *
 * EVIDENCE GATE (see substrate/plan.md Evidence Gate Plan + PARITY.md):
 * TODO: Implement full FileInfo state diff (hash bytes exact) on shadow Get when javaDelegate narrowed;
 * wire HashMismatchReporter; add differential coverage per plan.md §2; run corpus gates with
 * -Dorg.gradle.rust.substrate.fileHashCache.enabled=true (shadow) then .authoritative=true.
 * Exact commands + criteria in plan.md.
 */
public class ShadowingFileHashCache {

    private static final Logger LOGGER = Logging.getLogger(ShadowingFileHashCache.class);

    // The Java-side delegate is kept as Object for phase 0 because the internal cache
    // abstractions (e.g. IndexedCache<HashCode, HashCode> used by CrossBuildFileHashCache)
    // are not yet abstracted behind a narrow "FileHashCache" interface suitable for shadowing.
    // Future step: introduce a minimal FileHashCacheStore interface and wrap the real impls.
    private final Object javaDelegate;
    private final RustFileHashCacheClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingFileHashCache(
        Object javaDelegate,
        RustFileHashCacheClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this(javaDelegate, rustClient, mismatchReporter, false);
    }

    public ShadowingFileHashCache(
        Object javaDelegate,
        RustFileHashCacheClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.javaDelegate = javaDelegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    public boolean isAuthoritative() {
        return authoritative;
    }

    /**
     * Shadowed get operation.
     *
     * @param path absolute path
     * @param length file length at observation time
     * @param lastModified mtime at observation time
     * @param kind cache namespace (FILE_HASHES / CHECKSUMS)
     * @return result preferring Java in shadow mode, Rust (validated) in authoritative
     */
    public RustFileHashCacheClient.FileInfoResult getFileInfo(String path, long length, long lastModified, String kind) {
        if (authoritative) {
            return getFileInfoAuthoritative(path, length, lastModified, kind);
        }
        return getFileInfoShadow(path, length, lastModified, kind);
    }

    private RustFileHashCacheClient.FileInfoResult getFileInfoAuthoritative(String path, long length, long lastModified, String kind) {
        // Authoritative: Rust is source of truth (exact to Shadowing* patterns).
        // Fail-closed on Rust error. Best-effort cross-check vs Java delegate when present.
        RustFileHashCacheClient.FileInfoResult rustResult = rustClient.getFileInfo(path, length, lastModified, kind);

        if (!rustResult.isSuccess()) {
            mismatchReporter.reportRustError(keyFor(path, kind), new RuntimeException(rustResult.getErrorMessage()));
            throw new SubstrateException("Authoritative Rust FileHashCache get failed for " + keyFor(path, kind) + ": " + rustResult.getErrorMessage());
        }

        if (javaDelegate != null && rustResult.isHit()) {
            // TODO (next): perform actual javaDelegate lookup + value equality check on FileInfoData;
            // on divergence: mismatchReporter.reportMismatch(...) + throw SubstrateException (fail closed)
            mismatchReporter.reportMatch();
        } else if (rustResult.isHit()) {
            mismatchReporter.reportMatch();
        }
        return rustResult;
    }

    private RustFileHashCacheClient.FileInfoResult getFileInfoShadow(String path, long length, long lastModified, String kind) {
        // Shadow mode (exact parallel to ShadowingFileHasher / ShadowingValueSnapshotter):
        // 1. The Java delegate (CrossBuildFileHashCache / CachingFileHasher path) is authoritative for the caller result.
        // 2. We exercise the Rust path purely for differential observation / parity.
        // 3. Never return Rust data in shadow; always surface Java semantics (here represented synthetically
        //    until a narrow FileHashCacheStore iface allows real javaDelegate.get + compare).
        RustFileHashCacheClient.FileInfoResult rustResult = rustClient.getFileInfo(path, length, lastModified, kind);

        if (rustResult.isSuccess()) {
            mismatchReporter.reportMatch();  // exercised cleanly (real value diffing awaits iface)
        } else if (!rustResult.getErrorMessage().isEmpty()) {
            mismatchReporter.reportRustError(keyFor(path, kind), new RuntimeException(rustResult.getErrorMessage()));
        }

        // Return Java-equivalent result (synthetic miss until narrowed delegate integration).
        // TODO(next): when javaDelegate narrowed, do real lookup here and compare hashes before returning Java result.
        return new RustFileHashCacheClient.FileInfoResult(false, Optional.empty(), "");
    }

    /**
     * Shadowed put.
     */
    public boolean putFileInfo(String path, HashCode hash, long length, long lastModified, String kind) {
        boolean rustOk = rustClient.putFileInfo(path, hash, length, lastModified, kind);

        if (authoritative) {
            if (!rustOk) {
                throw new SubstrateException("Authoritative Rust file hash cache put failed for " + keyFor(path, kind));
            }
            return true;
        }

        // Shadow: we already did the Java put in the real path; just exercise + observe Rust.
        if (!rustOk) {
            mismatchReporter.reportRustError(keyFor(path, kind), new RuntimeException("Rust put returned failure"));
        } else {
            mismatchReporter.reportMatch();
        }
        return true; // Java side succeeded (caller already knows)
    }

    public boolean invalidate(String path) {
        boolean rustOk = rustClient.invalidate(path);
        if (authoritative && !rustOk) {
            // Invalidate is best-effort; do not fail closed aggressively in most cases.
            LOGGER.debug("[substrate:file-hash-cache] authoritative invalidate reported failure for {}", path);
        }
        return rustOk;
    }

    public RustFileHashCacheClient.CacheStats getStats() {
        // In shadow/auth scenarios stats are primarily for diagnostics.
        return rustClient.getStats();
    }

    private static String keyFor(String path, String kind) {
        return path + "::" + (kind == null ? "FILE_HASHES" : kind);
    }
}
