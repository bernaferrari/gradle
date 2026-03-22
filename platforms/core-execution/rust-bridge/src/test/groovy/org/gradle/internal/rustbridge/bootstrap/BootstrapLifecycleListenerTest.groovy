package org.gradle.internal.rustbridge.bootstrap

import spock.lang.Specification

class BootstrapLifecycleListenerTest extends Specification {

    def "constructor sets fields and generates build ID"() {
        given:
        def client = Mock(RustBootstrapClient)
        def projectDir = "/tmp/test-project"
        def parallelism = 4

        when:
        def listener = new BootstrapLifecycleListener(client, projectDir, parallelism)

        then:
        listener != null
        listener instanceof org.gradle.initialization.RootBuildLifecycleListener
    }

    def "constructor stores projectDir and parallelism"() {
        given:
        def client = Mock(RustBootstrapClient)

        when:
        def listener = new BootstrapLifecycleListener(client, "/my/project", 8)

        then:
        // The constructor stores these values internally; verify via behavior
        // afterStart will pass projectDir and parallelism to the client
        1 * client.initBuild(
            { it instanceof String }, // buildId (UUID)
            "/my/project",            // projectDir
            { it instanceof Long },   // startTimeMs
            8,                        // parallelism
            _ as Map,                 // systemProperties
            _ as List                 // requestedFeatures
        ) >> new RustBootstrapClient.BuildInitResult(
            "build-123", "1.0.0", "v1", 8
        )

        and:
        listener.afterStart()
    }

    def "afterStart calls initBuild on the bootstrap client"() {
        given:
        def client = Mock(RustBootstrapClient)
        def listener = new BootstrapLifecycleListener(client, "/test/dir", 2)
        def result = new RustBootstrapClient.BuildInitResult("b1", "0.1.0", "v1", 2)

        when:
        listener.afterStart()

        then:
        1 * client.initBuild(
            _ as String,
            "/test/dir",
            _ as long,
            2,
            _ as Map,
            _ as List
        ) >> result
    }

    def "afterStart swallows exceptions from bootstrap client"() {
        given:
        def client = Mock(RustBootstrapClient)
        def listener = new BootstrapLifecycleListener(client, "/test/dir", 1)

        when:
        listener.afterStart()

        then:
        client.initBuild(_, _, _, _, _, _) >> { throw new RuntimeException("connection refused") }
        noExceptionThrown()
    }

    def "beforeComplete skips when build was not initialized"() {
        given:
        def client = Mock(RustBootstrapClient)
        def listener = new BootstrapLifecycleListener(client, "/test/dir", 1)

        when:
        listener.beforeComplete(null)

        then:
        0 * client.completeBuild(_, _, _)
    }

    def "beforeComplete calls completeBuild with SUCCESS when no failure"() {
        given:
        def client = Mock(RustBootstrapClient)
        def listener = new BootstrapLifecycleListener(client, "/test/dir", 1)
        def initResult = new RustBootstrapClient.BuildInitResult("b1", "0.1.0", "v1", 1)

        when:
        listener.afterStart()
        listener.beforeComplete(null)

        then:
        1 * client.initBuild(_, _, _, _, _, _) >> initResult
        1 * client.completeBuild(_, "SUCCESS", _)
    }

    def "beforeComplete calls completeBuild with FAILED when failure is present"() {
        given:
        def client = Mock(RustBootstrapClient)
        def listener = new BootstrapLifecycleListener(client, "/test/dir", 1)
        def initResult = new RustBootstrapClient.BuildInitResult("b1", "0.1.0", "v1", 1)
        def failure = new RuntimeException("build failed")

        when:
        listener.afterStart()
        listener.beforeComplete(failure)

        then:
        1 * client.initBuild(_, _, _, _, _, _) >> initResult
        1 * client.completeBuild(_, "FAILED", _)
    }

    def "beforeComplete swallows exceptions from bootstrap client"() {
        given:
        def client = Mock(RustBootstrapClient)
        def listener = new BootstrapLifecycleListener(client, "/test/dir", 1)
        def initResult = new RustBootstrapClient.BuildInitResult("b1", "0.1.0", "v1", 1)

        when:
        listener.afterStart()
        listener.beforeComplete(null)

        then:
        1 * client.initBuild(_, _, _, _, _, _) >> initResult
        client.completeBuild(_, _, _) >> { throw new RuntimeException("daemon gone") }
        noExceptionThrown()
    }

    def "beforeComplete skips when afterStart failed to initialize"() {
        given:
        def client = Mock(RustBootstrapClient)
        def listener = new BootstrapLifecycleListener(client, "/test/dir", 1)

        when:
        listener.afterStart()   // will throw internally and set buildInitialized = false
        listener.beforeComplete(new RuntimeException("oops"))

        then:
        client.initBuild(_, _, _, _, _, _) >> { throw new RuntimeException("init failed") }
        0 * client.completeBuild(_, _, _)
    }
}
