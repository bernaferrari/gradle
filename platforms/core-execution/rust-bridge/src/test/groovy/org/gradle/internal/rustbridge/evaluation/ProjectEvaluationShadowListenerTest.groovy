package org.gradle.internal.rustbridge.evaluation

import org.gradle.api.Project
import org.gradle.api.ProjectEvaluationListener
import org.gradle.api.ProjectState
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class ProjectEvaluationShadowListenerTest extends Specification {

    def "implements ProjectEvaluationListener"() {
        given:
        def client = Mock(SubstrateClient)
        def listener = new ProjectEvaluationShadowListener(client)

        expect:
        listener instanceof ProjectEvaluationListener
    }

    def "beforeEvaluate and afterEvaluate track evaluation count"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new ProjectEvaluationShadowListener(client)

        def project = Mock(Project)
        project.getPath() >> ":app"
        def state = Mock(ProjectState)
        state.getFailure() >> null

        when:
        listener.beforeEvaluate(project)
        listener.afterEvaluate(project, state)

        then:
        1 * eventStreamStub.sendBuildEvent(_)
        listener.getEvaluatedCount() == 1
        listener.getFailedCount() == 0
    }

    def "afterEvaluate computes duration correctly"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new ProjectEvaluationShadowListener(client)

        def project = Mock(Project)
        project.getPath() >> ":lib"
        def state = Mock(ProjectState)
        state.getFailure() >> null

        when:
        listener.beforeEvaluate(project)
        Thread.sleep(50)
        listener.afterEvaluate(project, state)

        then:
        listener.getEvaluatedCount() == 1
        listener.getTotalEvalDurationMs() >= 40L
        listener.getSlowestEvalMs() >= 40L
        listener.getSlowestProject() == ":lib"
    }

    def "afterEvaluate tracks failed evaluations when state has failure"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new ProjectEvaluationShadowListener(client)

        def project = Mock(Project)
        project.getPath() >> ":failing"
        def state = Mock(ProjectState)
        state.getFailure() >> new RuntimeException("build script error")

        when:
        listener.beforeEvaluate(project)
        listener.afterEvaluate(project, state)

        then:
        listener.getEvaluatedCount() == 1
        listener.getFailedCount() == 1
    }

    def "afterEvaluate tracks slowest project"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new ProjectEvaluationShadowListener(client)

        def fastProject = Mock(Project)
        fastProject.getPath() >> ":fast"
        def slowProject = Mock(Project)
        slowProject.getPath() >> ":slow"
        def state = Mock(ProjectState)
        state.getFailure() >> null

        when:
        listener.beforeEvaluate(fastProject)
        listener.afterEvaluate(fastProject, state)
        listener.beforeEvaluate(slowProject)
        Thread.sleep(50)
        listener.afterEvaluate(slowProject, state)

        then:
        listener.getEvaluatedCount() == 2
        listener.getSlowestProject() == ":slow"
    }

    def "afterEvaluate without beforeEvaluate is a no-op"() {
        given:
        def client = Mock(SubstrateClient)
        def listener = new ProjectEvaluationShadowListener(client)

        def project = Mock(Project)
        project.getPath() >> ":orphan"
        def state = Mock(ProjectState)
        state.getFailure() >> null

        when:
        listener.afterEvaluate(project, state)

        then:
        listener.getEvaluatedCount() == 0
        listener.getFailedCount() == 0
        listener.getTotalEvalDurationMs() == 0
        listener.getSlowestEvalMs() == 0
        listener.getSlowestProject() == ""
    }

    def "metrics accumulators work correctly across multiple evaluations"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new ProjectEvaluationShadowListener(client)

        def projectA = Mock(Project)
        projectA.getPath() >> ":a"
        def projectB = Mock(Project)
        projectB.getPath() >> ":b"
        def projectC = Mock(Project)
        projectC.getPath() >> ":c"

        def okState = Mock(ProjectState)
        okState.getFailure() >> null
        def failState = Mock(ProjectState)
        failState.getFailure() >> new RuntimeException("error in :c")

        when:
        listener.beforeEvaluate(projectA)
        listener.afterEvaluate(projectA, okState)

        listener.beforeEvaluate(projectB)
        listener.afterEvaluate(projectB, okState)

        listener.beforeEvaluate(projectC)
        listener.afterEvaluate(projectC, failState)

        then:
        listener.getEvaluatedCount() == 3
        listener.getFailedCount() == 1
        listener.getTotalEvalDurationMs() >= 0L
        listener.getSlowestProject() != null
    }
}
