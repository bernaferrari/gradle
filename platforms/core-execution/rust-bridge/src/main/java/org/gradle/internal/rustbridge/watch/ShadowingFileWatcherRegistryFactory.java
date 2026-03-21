package org.gradle.internal.rustbridge.watch;

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.watch.registry.FileWatcherRegistry;
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory;

/**
 * A {@link FileWatcherRegistryFactory} that wraps the real factory and creates
 * {@link ShadowingFileWatcherRegistry} instances that shadow against Rust.
 */
public class ShadowingFileWatcherRegistryFactory implements FileWatcherRegistryFactory {

    private final FileWatcherRegistryFactory delegate;
    private final RustFileWatchClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingFileWatcherRegistryFactory(
        FileWatcherRegistryFactory delegate,
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.delegate = delegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    @Override
    public FileWatcherRegistry createFileWatcherRegistry(FileWatcherRegistry.ChangeHandler handler) {
        FileWatcherRegistry realRegistry = delegate.createFileWatcherRegistry(handler);
        return new ShadowingFileWatcherRegistry(realRegistry, rustClient, mismatchReporter);
    }
}
