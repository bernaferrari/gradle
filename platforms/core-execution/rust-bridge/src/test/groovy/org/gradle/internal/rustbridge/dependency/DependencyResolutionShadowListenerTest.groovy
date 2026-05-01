package org.gradle.internal.rustbridge.dependency

import org.gradle.api.artifacts.ArtifactCollection
import org.gradle.api.artifacts.ResolvableDependencies
import org.gradle.api.artifacts.component.ComponentArtifactIdentifier
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import org.gradle.api.artifacts.result.ResolvedArtifactResult
import org.gradle.api.artifacts.result.ResolvedComponentResult
import org.gradle.api.artifacts.result.ResolutionResult
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.junit.Rule
import org.junit.rules.TemporaryFolder
import spock.lang.Specification

class DependencyResolutionShadowListenerTest extends Specification {
    @Rule
    TemporaryFolder temporaryFolder = new TemporaryFolder()

    def "constructor sets up client and mismatchReporter fields"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def listener = new DependencyResolutionShadowListener(client, reporter)

        then:
        listener.totalResolutionTimeMs == 0
        listener.resolutionCount == 0
    }

    def "beforeResolve records start time and does not call client"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "compileClasspath"
        }

        when:
        listener.beforeResolve(dependencies)

        then:
        0 * client._
        0 * reporter._
    }

    def "beforeResolve is a no-op when client is null"() {
        given:
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(null, reporter)
        def dependencies = Mock(ResolvableDependencies)

        when:
        listener.beforeResolve(dependencies)

        then:
        noExceptionThrown()
    }

    def "afterResolve records resolution and reports match on success"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def componentA = Mock(ResolvedComponentResult)
        def componentB = Mock(ResolvedComponentResult)
        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([componentA, componentB] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
        }

        // Call beforeResolve first so the start time is recorded
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * client.recordResolution("runtimeClasspath", _, 2, true, 0)
        1 * reporter.reportMatch()
        listener.resolutionCount == 1
        listener.totalResolutionTimeMs >= 0
    }

    def "afterResolve reports rust error when client throws exception"() {
        given:
        def client = Mock(RustDependencyResolutionClient) {
            recordResolution(*_) >> { throw new RuntimeException("gRPC failure") }
        }
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "testRuntimeClasspath"
            getResolutionResult() >> resolutionResult
        }

        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * reporter.reportRustError("dep-resolve:testRuntimeClasspath", _)
    }

    def "afterResolve is a no-op when client is null"() {
        given:
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(null, reporter)
        def dependencies = Mock(ResolvableDependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        0 * reporter._
        noExceptionThrown()
    }

    def "afterResolve handles resolution result extraction failure gracefully"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "api"
            getResolutionResult() >> { throw new RuntimeException("resolution incomplete") }
        }

        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * client.recordResolution("api", _, 0, true, 0)
        1 * reporter.reportMatch()
    }

    def "resolution count and total time accumulate across multiple resolutions"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }

        def depsCompile = Mock(ResolvableDependencies) {
            getName() >> "compileClasspath"
            getResolutionResult() >> resolutionResult
        }
        def depsRuntime = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
        }

        when:
        listener.beforeResolve(depsCompile)
        listener.afterResolve(depsCompile)

        listener.beforeResolve(depsRuntime)
        listener.afterResolve(depsRuntime)

        then:
        listener.resolutionCount == 2
        listener.totalResolutionTimeMs >= 0
        2 * reporter.reportMatch()
    }

    def "authoritative mode records via strict call"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter, true)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
        }
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        listener.isAuthoritative()
        1 * client.recordResolutionStrict("runtimeClasspath", _, 0, true, 0)
        1 * reporter.reportMatch()
    }

    def "authoritative mode reports rust error when strict recording fails"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter, true)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "compileClasspath"
            getResolutionResult() >> resolutionResult
        }
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * client.recordResolutionStrict("compileClasspath", _, 0, true, 0) >> {
            throw new RuntimeException("rpc down")
        }
        1 * reporter.reportRustError("dep-resolve:compileClasspath", _ as RuntimeException)
        0 * client.recordResolution("compileClasspath", _, 0, true, 0)
    }

    def "mirror mode copies resolved module artifacts into rust cache"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter, false, true)
        def jar = temporaryFolder.newFile("demo-1.2.3-sources.jar")
        jar.text = "artifact bytes"

        def moduleId = Mock(ModuleComponentIdentifier) {
            getGroup() >> "org.example"
            getModule() >> "demo"
            getVersion() >> "1.2.3"
        }
        def artifactId = Mock(ComponentArtifactIdentifier) {
            getComponentIdentifier() >> moduleId
            getDisplayName() >> "demo sources"
        }
        def artifact = Mock(ResolvedArtifactResult) {
            getId() >> artifactId
            getFile() >> jar
        }
        def artifacts = Mock(ArtifactCollection) {
            getArtifacts() >> ([artifact] as Set)
        }
        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
            getArtifacts() >> artifacts
        }
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        listener.isMirrorArtifacts()
        listener.mirroredArtifactCount == 1
        1 * client.addArtifactToCache(
            "org.example",
            "demo",
            "1.2.3",
            "sources",
            "jar",
            jar.absolutePath,
            jar.length(),
            DependencyResolutionShadowListener.sha256(jar)
        ) >> true
        1 * client.recordResolution("runtimeClasspath", _, 0, true, 0)
        1 * reporter.reportMatch()
        0 * reporter.reportRustError(_, _)
    }

    def "mirror mode copies non-jar module artifacts with their extension"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter, false, true)
        def aar = temporaryFolder.newFile("demo-1.2.3-debug.aar")
        aar.text = "artifact bytes"

        def moduleId = Mock(ModuleComponentIdentifier) {
            getGroup() >> "org.example"
            getModule() >> "demo"
            getVersion() >> "1.2.3"
        }
        def artifactId = Mock(ComponentArtifactIdentifier) {
            getComponentIdentifier() >> moduleId
            getDisplayName() >> "demo debug aar"
        }
        def artifact = Mock(ResolvedArtifactResult) {
            getId() >> artifactId
            getFile() >> aar
        }
        def artifacts = Mock(ArtifactCollection) {
            getArtifacts() >> ([artifact] as Set)
        }
        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
            getArtifacts() >> artifacts
        }
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        listener.mirroredArtifactCount == 1
        1 * client.addArtifactToCache(
            "org.example",
            "demo",
            "1.2.3",
            "debug",
            "aar",
            aar.absolutePath,
            aar.length(),
            DependencyResolutionShadowListener.sha256(aar)
        ) >> true
        1 * client.recordResolution("runtimeClasspath", _, 0, true, 0)
        1 * reporter.reportMatch()
        0 * reporter.reportRustError(_, _)
    }

    def "mirror mode ignores project artifacts"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter, false, true)
        def txt = temporaryFolder.newFile("demo-1.2.3.txt")
        txt.text = "not a jar"

        def artifactId = Mock(ComponentArtifactIdentifier) {
            getComponentIdentifier() >> Mock(org.gradle.api.artifacts.component.ProjectComponentIdentifier)
            getDisplayName() >> "project artifact"
        }
        def artifact = Mock(ResolvedArtifactResult) {
            getId() >> artifactId
            getFile() >> txt
        }
        def artifacts = Mock(ArtifactCollection) {
            getArtifacts() >> ([artifact] as Set)
        }
        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
            getArtifacts() >> artifacts
        }
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        listener.mirroredArtifactCount == 0
        0 * client.addArtifactToCache(_, _, _, _, _, _, _, _)
        1 * client.recordResolution("runtimeClasspath", _, 0, true, 0)
        1 * reporter.reportMatch()
    }

    def "classifier inference handles main and classified jars"() {
        expect:
        DependencyResolutionShadowListener.inferClassifier(fileName, "demo", "1.2.3") == classifier

        where:
        fileName                    | classifier
        "demo-1.2.3.jar"            | ""
        "demo-1.2.3-sources.jar"    | "sources"
        "other-1.2.3.jar"           | ""
        "demo-1.2.3.module"         | ""
    }

    def "extension inference handles common module artifact names"() {
        expect:
        DependencyResolutionShadowListener.inferExtension(fileName) == extension

        where:
        fileName                    | extension
        "demo-1.2.3.jar"            | "jar"
        "demo-1.2.3-debug.aar"      | "aar"
        "demo-1.2.3.pom"            | "pom"
        "README"                    | ""
    }
}
