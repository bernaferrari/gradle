package org.gradle.internal.rustbridge.watch

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.internal.watch.registry.FileWatcherRegistry
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory
import spock.lang.Specification

class ShadowingFileWatcherRegistryFactoryTest extends Specification {

    def "implements FileWatcherRegistryFactory"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)

        expect:
        new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter, false) instanceof FileWatcherRegistryFactory
    }

    def "constructor stores delegate and Rust client"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter, false)

        then:
        factory != null
    }

    def "createFileWatcherRegistry delegates to the real factory and wraps result"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def realRegistry = Mock(FileWatcherRegistry)
        def handler = Mock(FileWatcherRegistry.ChangeHandler)
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter, false)

        when:
        def result = factory.createFileWatcherRegistry(handler)

        then:
        1 * delegate.createFileWatcherRegistry(_ as FileWatcherRegistry.ChangeHandler) >> realRegistry
        result instanceof ShadowingFileWatcherRegistry
    }

    def "wrapped change handler forwards events to provided handler"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def realRegistry = Mock(FileWatcherRegistry)
        def handler = Mock(FileWatcherRegistry.ChangeHandler)
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter, false)
        FileWatcherRegistry.ChangeHandler captured

        when:
        factory.createFileWatcherRegistry(handler)
        captured.handleChange(FileWatcherRegistry.Type.MODIFIED, java.nio.file.Paths.get("/tmp/f"))
        captured.stopWatchingAfterError()

        then:
        1 * delegate.createFileWatcherRegistry(_ as FileWatcherRegistry.ChangeHandler) >> {
            captured = it[0]
            realRegistry
        }
        1 * handler.handleChange(FileWatcherRegistry.Type.MODIFIED, java.nio.file.Paths.get("/tmp/f"))
        1 * handler.stopWatchingAfterError()
    }

    def "createFileWatcherRegistry returns new instance on each call"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def realRegistry = Mock(FileWatcherRegistry)
        def handler = Mock(FileWatcherRegistry.ChangeHandler)
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter, false)

        when:
        def first = factory.createFileWatcherRegistry(handler)
        def second = factory.createFileWatcherRegistry(handler)

        then:
        2 * delegate.createFileWatcherRegistry(_ as FileWatcherRegistry.ChangeHandler) >> realRegistry
        first != second
        first instanceof ShadowingFileWatcherRegistry
        second instanceof ShadowingFileWatcherRegistry
    }

    def "createFileWatcherRegistry propagates authoritative mode to registry"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def realRegistry = Mock(FileWatcherRegistry)
        def handler = Mock(FileWatcherRegistry.ChangeHandler)
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter, true)

        when:
        def result = factory.createFileWatcherRegistry(handler) as ShadowingFileWatcherRegistry

        then:
        1 * delegate.createFileWatcherRegistry(_ as FileWatcherRegistry.ChangeHandler) >> realRegistry
        result.isAuthoritative()
    }
}
