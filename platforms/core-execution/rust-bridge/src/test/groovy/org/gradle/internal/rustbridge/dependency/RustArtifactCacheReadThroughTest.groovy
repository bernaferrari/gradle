package org.gradle.internal.rustbridge.dependency

import org.junit.Rule
import org.junit.rules.TemporaryFolder
import spock.lang.Specification

class RustArtifactCacheReadThroughTest extends Specification {
    @Rule
    TemporaryFolder temporaryFolder = new TemporaryFolder()

    def "resolves complete artifact coordinates from rust cache"() {
        given:
        def cachedJar = temporaryFolder.newFile("demo-1.2.3-sources.jar")
        cachedJar.text = "jar bytes"
        def client = Mock(RustDependencyResolutionClient)
        def readThrough = new RustArtifactCacheReadThrough(client)

        when:
        def result = readThrough.findCachedArtifact("org.example", "demo", "1.2.3", "sources", "jar")

        then:
        result == cachedJar
        1 * client.checkArtifactCache("org.example", "demo", "1.2.3", "sources", "jar", "") >>
            RustDependencyResolutionClient.CacheCheckResult.cached(cachedJar.absolutePath, cachedJar.length())
        0 * client._
    }

    def "falls back closed for missing rust cache path"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def readThrough = new RustArtifactCacheReadThrough(client)

        when:
        def result = readThrough.findCachedArtifact("org.example", "demo", "1.2.3", "", "jar")

        then:
        result == null
        1 * client.checkArtifactCache("org.example", "demo", "1.2.3", "", "jar", "") >>
            RustDependencyResolutionClient.CacheCheckResult.notCached()
        0 * client._
    }

    def "falls back closed when rust returns a non-file path"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def readThrough = new RustArtifactCacheReadThrough(client)

        when:
        def result = readThrough.findCachedArtifact("org.example", "demo", "1.2.3", "", "jar")

        then:
        result == null
        1 * client.checkArtifactCache("org.example", "demo", "1.2.3", "", "jar", "") >>
            RustDependencyResolutionClient.CacheCheckResult.cached(new File(temporaryFolder.root, "missing.jar").absolutePath, 12)
        0 * client._
    }
}
