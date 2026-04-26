package org.gradle.internal.rustbridge.exec

import org.gradle.internal.rustbridge.SubstrateClient
import org.gradle.internal.rustbridge.SubstrateException
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.process.ExecResult
import org.gradle.process.ProcessExecutionException
import org.gradle.process.internal.ExecAction
import org.gradle.process.internal.ExecActionFactory
import spock.lang.Specification

class ShadowingExecActionFactoryTest extends Specification {

    def javaFactory = Mock(ExecActionFactory)
    def javaAction = Mock(ExecAction)
    def rustAction = Mock(ExecAction)
    def client = Mock(SubstrateClient)
    def reporter = Mock(HashMismatchReporter)

    def "shadow mode returns Java exec action when substrate is no-op"() {
        given:
        def factory = new ShadowingExecActionFactory(javaFactory, client, reporter, false)

        when:
        def action = factory.newExecAction()

        then:
        1 * client.isNoop() >> true
        1 * javaFactory.newExecAction() >> javaAction
        action.is(javaAction)
    }

    def "authoritative mode fails when substrate is no-op"() {
        given:
        def factory = new ShadowingExecActionFactory(javaFactory, client, reporter, true)

        when:
        factory.newExecAction()

        then:
        1 * client.isNoop() >> true
        1 * client.getNoopReason() >> "disabled"
        def failure = thrown(SubstrateException)
        failure.message == "Authoritative Rust exec is unavailable: disabled"
        0 * javaFactory.newExecAction()
    }

    def "authoritative mode returns Rust exec result without invoking Java execution"() {
        given:
        def result = Mock(ExecResult)
        def factory = factoryWithRustAction(true)
        configureJavaAction()

        when:
        def actual = factory.newExecAction().execute()

        then:
        1 * client.isNoop() >> false
        1 * javaFactory.newExecAction() >> javaAction
        1 * rustAction.execute() >> result
        0 * javaAction.execute()
        actual.is(result)
    }

    def "authoritative mode propagates Rust execution failure without invoking Java execution"() {
        given:
        def failure = new ProcessExecutionException("rust spawn failed")
        def factory = factoryWithRustAction(true)
        configureJavaAction()

        when:
        factory.newExecAction().execute()

        then:
        1 * client.isNoop() >> false
        1 * javaFactory.newExecAction() >> javaAction
        1 * rustAction.execute() >> { throw failure }
        1 * reporter.reportRustError("exec:[echo, ok]", failure)
        0 * javaAction.execute()
        def thrownFailure = thrown(ProcessExecutionException)
        thrownFailure.is(failure)
    }

    def "authoritative mode fails when Java config cannot sync to Rust"() {
        given:
        def factory = factoryWithRustAction(true)

        when:
        factory.newExecAction().execute()

        then:
        1 * client.isNoop() >> false
        1 * javaFactory.newExecAction() >> javaAction
        2 * javaAction.getCommandLine() >> { throw new RuntimeException("bad command line") }
        1 * reporter.reportRustError("exec:unknown", _ as IllegalStateException)
        0 * rustAction.execute()
        0 * javaAction.execute()
        def failure = thrown(ProcessExecutionException)
        failure.message == "Authoritative Rust exec failed for unknown"
    }

    private ShadowingExecActionFactory factoryWithRustAction(boolean authoritative) {
        new ShadowingExecActionFactory(
            javaFactory,
            client,
            reporter,
            authoritative,
            { SubstrateClient ignored -> rustAction } as ShadowingExecActionFactory.RustActionFactory
        )
    }

    private void configureJavaAction() {
        javaAction.getCommandLine() >> ["echo", "ok"]
        javaAction.getWorkingDir() >> null
        javaAction.getEnvironment() >> [:]
        javaAction.isIgnoreExitValue() >> false
        javaAction.getStandardOutput() >> null
        javaAction.getErrorOutput() >> null
    }
}
