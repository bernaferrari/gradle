package org.gradle.internal.rustbridge.cache

import org.gradle.caching.BuildCacheServiceFactory
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class RustBuildCacheServiceFactoryTest extends Specification {

    def "creates RustBuildCacheService and populates describer"() {
        given:
        def client = Mock(SubstrateClient)
        def describer = Mock(BuildCacheServiceFactory.Describer)
        def configuration = Mock(RustBuildCache)
        def factory = new RustBuildCacheServiceFactory(client)

        when:
        def service = factory.createBuildCacheService(configuration, describer)

        then:
        service instanceof RustBuildCacheService
        1 * describer.type("Rust Local")
        1 * describer.config("backend", "rust-local-filesystem")
    }
}

