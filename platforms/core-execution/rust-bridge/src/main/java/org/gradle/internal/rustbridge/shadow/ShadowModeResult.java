package org.gradle.internal.rustbridge.shadow;

/**
 * Result of comparing Java and Rust implementations in shadow mode.
 */
public enum ShadowModeResult {
    /** Both implementations agree. */
    MATCH,
    /** Implementations disagree — potential bug in Rust implementation. */
    MISMATCH,
    /** Rust threw an error, Java succeeded. */
    RUST_ERROR,
    /** Java threw an error, Rust succeeded. */
    JAVA_ERROR,
    /** Both implementations threw errors. */
    BOTH_ERROR
}
