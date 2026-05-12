package org.gradle.internal.rustbridge.watch

import org.gradle.internal.buildoption.RustSubstrateOptions
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory
import spock.lang.Specification

class RustFileWatchWiringTest extends Specification {

    def "leaves registry factory unchanged when Rust file watching is disabled"() {
        given:
        def factory = Mock(FileWatcherRegistryFactory)
        def client = Mock(RustFileWatchClient)

        when:
        def result = withFileWatchProperty(null) {
            RustFileWatchWiring.wrapIfEnabled(Optional.of(factory), client)
        }

        then:
        result.isPresent()
        result.get().is(factory)
    }

    def "leaves registry factory unchanged when user-home Rust client is unavailable"() {
        given:
        def factory = Mock(FileWatcherRegistryFactory)

        when:
        def result = withFileWatchProperty("true") {
            RustFileWatchWiring.wrapIfEnabled(Optional.of(factory), null)
        }

        then:
        result.isPresent()
        result.get().is(factory)
    }

    def "wraps registry factory with user-home Rust file-watch client when enabled"() {
        given:
        def factory = Mock(FileWatcherRegistryFactory)
        def client = Mock(RustFileWatchClient)

        when:
        def result = withFileWatchProperty("true") {
            RustFileWatchWiring.wrapIfEnabled(Optional.of(factory), client)
        }

        then:
        result.isPresent()
        result.get() instanceof ShadowingFileWatcherRegistryFactory
    }

    def "umbrella substrate mode enables user-home Rust file watching"() {
        given:
        def factory = Mock(FileWatcherRegistryFactory)
        def client = Mock(RustFileWatchClient)

        when:
        def result = withSubstrateMode("shadow") {
            RustFileWatchWiring.wrapIfEnabled(Optional.of(factory), client)
        }

        then:
        result.isPresent()
        result.get() instanceof ShadowingFileWatcherRegistryFactory
    }

    def "umbrella off mode disables user-home Rust file watching even when feature flag is set"() {
        given:
        def factory = Mock(FileWatcherRegistryFactory)
        def client = Mock(RustFileWatchClient)

        when:
        def result = withSubstrateMode("off") {
            withFileWatchProperty("true") {
                RustFileWatchWiring.wrapIfEnabled(Optional.of(factory), client)
            }
        }

        then:
        result.isPresent()
        result.get().is(factory)
    }

    def "does not create a watcher factory when platform watching is unavailable"() {
        given:
        def client = Mock(RustFileWatchClient)

        expect:
        !withFileWatchProperty("true") {
            RustFileWatchWiring.wrapIfEnabled(Optional.empty(), client)
        }.isPresent()
    }

    private static <T> T withFileWatchProperty(String value, Closure<T> action) {
        def key = RustSubstrateOptions.ENABLE_RUST_FILE_WATCH.propertyName
        def previous = System.getProperty(key)
        try {
            if (value == null) {
                System.clearProperty(key)
            } else {
                System.setProperty(key, value)
            }
            return action.call()
        } finally {
            if (previous == null) {
                System.clearProperty(key)
            } else {
                System.setProperty(key, previous)
            }
        }
    }

    private static <T> T withSubstrateMode(String value, Closure<T> action) {
        def key = RustSubstrateOptions.SUBSTRATE_MODE.propertyName
        def previous = System.getProperty(key)
        try {
            if (value == null) {
                System.clearProperty(key)
            } else {
                System.setProperty(key, value)
            }
            return action.call()
        } finally {
            if (previous == null) {
                System.clearProperty(key)
            } else {
                System.setProperty(key, previous)
            }
        }
    }
}
