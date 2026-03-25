package org.gradle.internal.rustbridge.cache

import org.gradle.caching.BuildCacheServiceFactory
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class RustRemoteBuildCacheServiceFactoryTest extends Specification {

    def "creates RustRemoteBuildCacheService and populates describer"() {
        given:
        def client = Mock(SubstrateClient)
        def describer = Mock(BuildCacheServiceFactory.Describer)
        def configuration = Mock(RustRemoteBuildCache)
        def factory = new RustRemoteBuildCacheServiceFactory(client)

        when:
        def service = factory.createBuildCacheService(configuration, describer)

        then:
        service instanceof RustRemoteBuildCacheService
        1 * describer.type("Rust Remote")
        1 * describer.config("backend", "rust-remote-cache")
    }
}

