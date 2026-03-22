package org.gradle.internal.rustbridge.watch

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.internal.watch.registry.FileWatcherRegistry
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory
import spock.lang.Specification

class ShadowingFileWatcherRegistryFactoryTest extends Specification {

    def "implements FileWatcherRegistryFactory"() {
        expect:
        ShadowingFileWatcherRegistryFactory instanceof FileWatcherRegistryFactory
    }

    def "constructor stores delegate and Rust client"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter)

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
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter)

        when:
        def result = factory.createFileWatcherRegistry(handler)

        then:
        1 * delegate.createFileWatcherRegistry(handler) >> realRegistry
        result instanceof ShadowingFileWatcherRegistry
    }

    def "createFileWatcherRegistry passes the same handler to delegate"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def realRegistry = Mock(FileWatcherRegistry)
        def handler = Mock(FileWatcherRegistry.ChangeHandler)
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter)

        when:
        factory.createFileWatcherRegistry(handler)

        then:
        1 * delegate.createFileWatcherRegistry(handler) >> realRegistry
    }

    def "createFileWatcherRegistry returns new instance on each call"() {
        given:
        def delegate = Mock(FileWatcherRegistryFactory)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def realRegistry = Mock(FileWatcherRegistry)
        def handler = Mock(FileWatcherRegistry.ChangeHandler)
        def factory = new ShadowingFileWatcherRegistryFactory(delegate, rustClient, reporter)

        when:
        def first = factory.createFileWatcherRegistry(handler)
        def second = factory.createFileWatcherRegistry(handler)

        then:
        2 * delegate.createFileWatcherRegistry(handler) >> realRegistry
        first != second
        first instanceof ShadowingFileWatcherRegistry
        second instanceof ShadowingFileWatcherRegistry
    }
}
