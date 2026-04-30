package org.gradle.internal.rustbridge.dependency

import org.junit.Rule
import org.junit.rules.TemporaryFolder
import spock.lang.Specification

class RustMetadataCacheReadThroughTest extends Specification {
    @Rule
    TemporaryFolder temporaryFolder = new TemporaryFolder()

    def "resolves cached metadata from rust cache"() {
        given:
        def cachedPom = temporaryFolder.newFile("demo-1.2.3.pom")
        cachedPom.text = "<project/>"
        def client = Mock(RustDependencyResolutionClient)
        def readThrough = new RustMetadataCacheReadThrough(client)
        def location = new URI("https://repo.example.test/org/example/demo/1.2.3/demo-1.2.3.pom")

        when:
        def result = readThrough.findCachedMetadata(location, "pom")

        then:
        result == cachedPom
        1 * client.checkMetadataCache(location.toString(), "pom", "") >>
            RustDependencyResolutionClient.CacheCheckResult.cached(cachedPom.absolutePath, cachedPom.length())
        0 * client._
    }

    def "falls back closed for missing rust metadata path"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def readThrough = new RustMetadataCacheReadThrough(client)
        def location = new URI("https://repo.example.test/org/example/demo/1.2.3/demo-1.2.3.pom")

        when:
        def result = readThrough.findCachedMetadata(location, "pom")

        then:
        result == null
        1 * client.checkMetadataCache(location.toString(), "pom", "") >>
            RustDependencyResolutionClient.CacheCheckResult.notCached()
        0 * client._
    }

    def "falls back closed when rust returns a non-file path"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def readThrough = new RustMetadataCacheReadThrough(client)
        def location = new URI("https://repo.example.test/org/example/demo/1.2.3/demo-1.2.3.pom")

        when:
        def result = readThrough.findCachedMetadata(location, "pom")

        then:
        result == null
        1 * client.checkMetadataCache(location.toString(), "pom", "") >>
            RustDependencyResolutionClient.CacheCheckResult.cached(new File(temporaryFolder.root, "missing.pom").absolutePath, 12)
        0 * client._
    }
}
