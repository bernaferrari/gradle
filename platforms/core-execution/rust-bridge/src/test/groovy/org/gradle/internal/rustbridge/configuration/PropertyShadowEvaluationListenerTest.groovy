package org.gradle.internal.rustbridge.configuration

import org.gradle.api.Project
import org.gradle.api.ProjectState
import org.gradle.api.plugins.Plugin
import spock.lang.Specification

class PropertyShadowEvaluationListenerTest extends Specification {

    def "constructor stores the property resolver"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)

        when:
        def listener = new PropertyShadowEvaluationListener(resolver)

        then:
        listener != null
    }

    def "beforeEvaluate is a no-op"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)
        def listener = new PropertyShadowEvaluationListener(resolver)
        def project = Mock(Project)

        when:
        listener.beforeEvaluate(project)

        then:
        0 * resolver._
    }

    def "afterEvaluate skips projects with evaluation failure"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)
        def listener = new PropertyShadowEvaluationListener(resolver)
        def project = Mock(Project)
        def state = Mock(ProjectState)
        state.getFailure() >> new RuntimeException("evaluation failed")

        when:
        listener.afterEvaluate(project, state)

        then:
        0 * resolver._
    }

    def "afterEvaluate registers project and shadow-resolves properties"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)
        def listener = new PropertyShadowEvaluationListener(resolver)
        def project = Mock(Project)
        def state = Mock(ProjectState)
        def plugin = Mock(Plugin)
        def projectDir = new File("/tmp/test-project")

        project.getPath() >> ":"
        project.getProjectDir() >> projectDir
        project.getProperties() >> ["version": "1.0", "group": "com.example"]
        project.getPlugins() >> [plugin]
        plugin.getClass() >> TestPlugin.class
        state.getFailure() >> null

        when:
        listener.afterEvaluate(project, state)

        then:
        1 * resolver.registerProject(
            ":",
            "/tmp/test-project",
            ["version": "1.0", "group": "com.example"],
            ["org.gradle.internal.rustbridge.configuration.TestPlugin"]
        )
        1 * resolver.shadowResolveProperty(":", "version", "1.0")
        1 * resolver.shadowResolveProperty(":", "group", "com.example")
    }

    def "afterEvaluate filters out null property values"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)
        def listener = new PropertyShadowEvaluationListener(resolver)
        def project = Mock(Project)
        def state = Mock(ProjectState)
        def projectDir = new File("/tmp/test-project")

        project.getPath() >> ":"
        project.getProjectDir() >> projectDir
        project.getProperties() >> ["version": "1.0", "nullable": null, "group": "com.example"]
        project.getPlugins() >> []
        state.getFailure() >> null

        when:
        listener.afterEvaluate(project, state)

        then:
        1 * resolver.registerProject(
            ":",
            "/tmp/test-project",
            ["version": "1.0", "group": "com.example"],
            []
        )
        1 * resolver.shadowResolveProperty(":", "version", "1.0")
        1 * resolver.shadowResolveProperty(":", "group", "com.example")
        0 * resolver.shadowResolveProperty(":", "nullable", _)
    }

    def "afterEvaluate handles empty properties and no plugins"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)
        def listener = new PropertyShadowEvaluationListener(resolver)
        def project = Mock(Project)
        def state = Mock(ProjectState)
        def projectDir = new File("/tmp/test-project")

        project.getPath() >> ":app"
        project.getProjectDir() >> projectDir
        project.getProperties() >> [:]
        project.getPlugins() >> []
        state.getFailure() >> null

        when:
        listener.afterEvaluate(project, state)

        then:
        1 * resolver.registerProject(":app", "/tmp/test-project", [:], [])
        0 * resolver.shadowResolveProperty(_, _, _)
    }

    def "afterEvaluate does not propagate exceptions from resolver"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)
        def listener = new PropertyShadowEvaluationListener(resolver)
        def project = Mock(Project)
        def state = Mock(ProjectState)
        def projectDir = new File("/tmp/test-project")

        project.getPath() >> ":"
        project.getProjectDir() >> projectDir
        project.getProperties() >> ["version": "1.0"]
        project.getPlugins() >> []
        state.getFailure() >> null

        resolver.registerProject(_, _, _, _) >> { throw new RuntimeException("rust connection failed") }

        when:
        listener.afterEvaluate(project, state)

        then:
        noExceptionThrown()
    }

    def "afterEvaluate does not propagate exceptions from shadowResolveProperty"() {
        given:
        def resolver = Mock(ShadowingPropertyResolver)
        def listener = new PropertyShadowEvaluationListener(resolver)
        def project = Mock(Project)
        def state = Mock(ProjectState)
        def projectDir = new File("/tmp/test-project")

        project.getPath() >> ":"
        project.getProjectDir() >> projectDir
        project.getProperties() >> ["version": "1.0"]
        project.getPlugins() >> []
        state.getFailure() >> null

        resolver.shadowResolveProperty(_, _, _) >> { throw new RuntimeException("shadow resolution failed") }

        when:
        listener.afterEvaluate(project, state)

        then:
        noExceptionThrown()
    }

    static class TestPlugin implements Plugin<Project> {
        @Override
        void apply(Project project) {}
    }
}
