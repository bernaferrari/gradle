package org.gradle.internal.rustbridge.watch;

import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.gradle.internal.buildoption.InternalOption;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory;
import org.jspecify.annotations.Nullable;

import java.util.Locale;
import java.util.Optional;

/**
 * Centralized wiring helper for Rust-backed file watching.
 */
public final class RustFileWatchWiring {

    private RustFileWatchWiring() {
    }

    public static Optional<FileWatcherRegistryFactory> wrapIfEnabled(
        Optional<FileWatcherRegistryFactory> maybeFactory,
        @Nullable RustFileWatchClient rustFileWatchClient
    ) {
        if (!isFileWatchEnabled()) {
            return maybeFactory;
        }
        if (!maybeFactory.isPresent() || rustFileWatchClient == null || rustFileWatchClient.isNoop()) {
            return maybeFactory;
        }

        boolean authoritative = isFileWatchAuthoritative();
        FileWatcherRegistryFactory shadowFactory = new ShadowingFileWatcherRegistryFactory(
            maybeFactory.get(),
            rustFileWatchClient,
            new HashMismatchReporter(true),
            authoritative
        );
        return Optional.of(shadowFactory);
    }

    private static boolean isFileWatchEnabled() {
        RustSubstrateOptions.SubstrateMode mode = systemMode();
        if (mode != null) {
            return mode != RustSubstrateOptions.SubstrateMode.OFF;
        }
        return isSystemPropertyEnabled(RustSubstrateOptions.ENABLE_RUST_FILE_WATCH);
    }

    private static boolean isFileWatchAuthoritative() {
        RustSubstrateOptions.SubstrateMode mode = systemMode();
        if (mode == RustSubstrateOptions.SubstrateMode.AUTHORITATIVE) {
            return true;
        }
        if (mode == RustSubstrateOptions.SubstrateMode.OFF) {
            return false;
        }
        return isSystemPropertyEnabled(RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_FILE_WATCH);
    }

    private static RustSubstrateOptions.SubstrateMode systemMode() {
        String value = System.getProperty(RustSubstrateOptions.SUBSTRATE_MODE.getPropertyName());
        if (value == null || value.trim().isEmpty()) {
            return null;
        }
        try {
            return RustSubstrateOptions.SubstrateMode.valueOf(value.trim().toUpperCase(Locale.ROOT));
        } catch (IllegalArgumentException e) {
            return null;
        }
    }

    private static boolean isSystemPropertyEnabled(InternalOption<Boolean> option) {
        String value = System.getProperty(option.getPropertyName());
        return value != null && Boolean.parseBoolean(value);
    }
}
