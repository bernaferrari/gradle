package org.gradle.internal.rustbridge.watch;

import org.gradle.internal.buildoption.InternalOptions;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory;
import org.jspecify.annotations.Nullable;

import java.util.Optional;

/**
 * Centralized wiring helper for Rust-backed file watching.
 */
public final class RustFileWatchWiring {

    private RustFileWatchWiring() {
    }

    public static Optional<FileWatcherRegistryFactory> wrapIfEnabled(
        Optional<FileWatcherRegistryFactory> maybeFactory,
        InternalOptions options,
        @Nullable SubstrateClient substrateClient
    ) {
        if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_FILE_WATCH)) {
            return maybeFactory;
        }
        if (!maybeFactory.isPresent() || substrateClient == null || substrateClient.isNoop()) {
            return maybeFactory;
        }

        boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
            options,
            RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_FILE_WATCH
        );
        FileWatcherRegistryFactory shadowFactory = new ShadowingFileWatcherRegistryFactory(
            maybeFactory.get(),
            new RustFileWatchClient(substrateClient),
            new HashMismatchReporter(true),
            authoritative
        );
        return Optional.of(shadowFactory);
    }
}

