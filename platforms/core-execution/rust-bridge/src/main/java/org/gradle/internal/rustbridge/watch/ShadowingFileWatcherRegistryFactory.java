package org.gradle.internal.rustbridge.watch;

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.watch.registry.FileWatcherRegistry;
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory;

import java.util.concurrent.atomic.AtomicReference;

/**
 * A {@link FileWatcherRegistryFactory} that wraps the real factory and creates
 * {@link ShadowingFileWatcherRegistry} instances that shadow against Rust.
 */
public class ShadowingFileWatcherRegistryFactory implements FileWatcherRegistryFactory {

    private final FileWatcherRegistryFactory delegate;
    private final RustFileWatchClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingFileWatcherRegistryFactory(
        FileWatcherRegistryFactory delegate,
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.delegate = delegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    @Override
    public FileWatcherRegistry createFileWatcherRegistry(FileWatcherRegistry.ChangeHandler handler) {
        AtomicReference<ShadowingFileWatcherRegistry> registryRef = new AtomicReference<>();
        FileWatcherRegistry.ChangeHandler shadowingHandler = new FileWatcherRegistry.ChangeHandler() {
            @Override
            public void handleChange(FileWatcherRegistry.Type type, java.nio.file.Path path) {
                ShadowingFileWatcherRegistry registry = registryRef.get();
                if (registry != null) {
                    registry.recordJavaChange();
                }
                handler.handleChange(type, path);
            }

            @Override
            public void stopWatchingAfterError() {
                handler.stopWatchingAfterError();
            }
        };

        FileWatcherRegistry realRegistry = delegate.createFileWatcherRegistry(shadowingHandler);
        ShadowingFileWatcherRegistry registry =
            new ShadowingFileWatcherRegistry(realRegistry, rustClient, mismatchReporter, authoritative);
        registryRef.set(registry);
        return registry;
    }
}
