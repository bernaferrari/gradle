package org.gradle.internal.rustbridge.testexec

import org.gradle.api.tasks.testing.TestDescriptor
import org.gradle.api.tasks.testing.TestListener
import org.gradle.api.tasks.testing.TestResult
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class TestExecutionShadowListenerTest extends Specification {

    def "implements TestListener"() {
        expect:
        TestExecutionShadowListener instanceof TestListener
    }

    def "constructor accepts SubstrateClient"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true

        when:
        def listener = new TestExecutionShadowListener(client)

        then:
        listener != null
    }

    def "beforeSuite is a no-op when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new TestExecutionShadowListener(client)
        def suite = Mock(TestDescriptor)

        when:
        listener.beforeSuite(suite)

        then:
        0 * client.getTestExecutionStub()
    }

    def "afterSuite is a no-op when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new TestExecutionShadowListener(client)
        def suite = Mock(TestDescriptor)
        def result = Mock(TestResult)

        when:
        listener.afterSuite(suite, result)

        then:
        0 * client.getTestExecutionStub()
    }

    def "beforeTest is always a no-op"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        def listener = new TestExecutionShadowListener(client)
        def testDescriptor = Mock(TestDescriptor)

        when:
        listener.beforeTest(testDescriptor)

        then:
        0 * client._
    }

    def "afterTest is a no-op when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new TestExecutionShadowListener(client)
        def testDescriptor = Mock(TestDescriptor)
        def result = Mock(TestResult)

        when:
        listener.afterTest(testDescriptor, result)

        then:
        0 * client.getTestExecutionStub()
    }

    def "afterTest delegates to client when not noop for successful test"() {
        given:
        def stub = Mock(gradle.substrate.v1.TestExecutionServiceGrpc.TestExecutionServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> stub
        def listener = new TestExecutionShadowListener(client)

        def testDescriptor = Mock(TestDescriptor)
        testDescriptor.getName() >> "testSomething"
        testDescriptor.getClassName() >> "com.example.MyTest"

        def result = Mock(TestResult)
        result.getResultType() >> TestResult.ResultType.SUCCESS
        result.getStartTime() >> 1000L
        result.getEndTime() >> 1500L
        result.getException() >> null

        when:
        listener.afterTest(testDescriptor, result)

        then:
        1 * stub.reportTestResult(_)
    }

    def "afterTest delegates to client when not noop for failed test"() {
        given:
        def stub = Mock(gradle.substrate.v1.TestExecutionServiceGrpc.TestExecutionServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> stub
        def listener = new TestExecutionShadowListener(client)

        def testDescriptor = Mock(TestDescriptor)
        testDescriptor.getName() >> "testFails"
        testDescriptor.getClassName() >> "com.example.FailingTest"

        def result = Mock(TestResult)
        result.getResultType() >> TestResult.ResultType.FAILURE
        result.getStartTime() >> 2000L
        result.getEndTime() >> 2500L
        result.getException() >> new AssertionError("expected true but was false")

        when:
        listener.afterTest(testDescriptor, result)

        then:
        1 * stub.reportTestResult(_)
    }

    def "afterTest delegates to client for skipped test"() {
        given:
        def stub = Mock(gradle.substrate.v1.TestExecutionServiceGrpc.TestExecutionServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> stub
        def listener = new TestExecutionShadowListener(client)

        def testDescriptor = Mock(TestDescriptor)
        testDescriptor.getName() >> "testSkipped"
        testDescriptor.getClassName() >> "com.example.SkippedTest"

        def result = Mock(TestResult)
        result.getResultType() >> TestResult.ResultType.SKIPPED
        result.getStartTime() >> 0L
        result.getEndTime() >> 0L
        result.getException() >> null

        when:
        listener.afterTest(testDescriptor, result)

        then:
        1 * stub.reportTestResult(_)
    }

    def "afterTest catches exception and does not propagate"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> { throw new RuntimeException("connection failed") }
        def listener = new TestExecutionShadowListener(client)

        def testDescriptor = Mock(TestDescriptor)
        testDescriptor.getName() >> "testBoom"
        testDescriptor.getClassName() >> "com.example.BoomTest"

        def result = Mock(TestResult)
        result.getResultType() >> TestResult.ResultType.SUCCESS
        result.getStartTime() >> 0L
        result.getEndTime() >> 100L
        result.getException() >> null

        when:
        listener.afterTest(testDescriptor, result)

        then:
        noExceptionThrown()
    }

    def "beforeSuite catches exception and does not propagate"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> { throw new RuntimeException("connection failed") }
        def listener = new TestExecutionShadowListener(client)

        def suite = Mock(TestDescriptor)
        suite.getName() >> "suite"
        suite.getClassName() >> "com.example.Suite"

        when:
        listener.beforeSuite(suite)

        then:
        noExceptionThrown()
    }

    def "afterSuite catches exception and does not propagate"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> { throw new RuntimeException("connection failed") }
        def listener = new TestExecutionShadowListener(client)

        def suite = Mock(TestDescriptor)
        suite.getName() >> "suite"
        suite.getParent() >> null

        def result = Mock(TestResult)
        result.getTestCount() >> 1

        when:
        listener.afterSuite(suite, result)

        then:
        noExceptionThrown()
    }

    def "afterSuite does not call detectFlakyTests when suite has a parent"() {
        given:
        def stub = Mock(gradle.substrate.v1.TestExecutionServiceGrpc.TestExecutionServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> stub
        def listener = new TestExecutionShadowListener(client)

        def parentSuite = Mock(TestDescriptor)
        def suite = Mock(TestDescriptor)
        suite.getName() >> "child suite"
        suite.getParent() >> parentSuite

        def result = Mock(TestResult)
        result.getTestCount() >> 5
        result.getSuccessfulTestCount() >> 3
        result.getFailedTestCount() >> 1
        result.getSkippedTestCount() >> 1

        when:
        listener.afterSuite(suite, result)

        then:
        0 * stub.detectFlakyTests(_)
    }

    def "afterSuite does not call detectFlakyTests when test count is zero"() {
        given:
        def stub = Mock(gradle.substrate.v1.TestExecutionServiceGrpc.TestExecutionServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> stub
        def listener = new TestExecutionShadowListener(client)

        def suite = Mock(TestDescriptor)
        suite.getName() >> "root suite"
        suite.getParent() >> null

        def result = Mock(TestResult)
        result.getTestCount() >> 0

        when:
        listener.afterSuite(suite, result)

        then:
        0 * stub.detectFlakyTests(_)
    }

    def "afterSuite calls detectFlakyTests on root suite with tests"() {
        given:
        def stub = Mock(gradle.substrate.v1.TestExecutionServiceGrpc.TestExecutionServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getTestExecutionStub() >> stub
        def listener = new TestExecutionShadowListener(client)

        def flakyResponse = gradle.substrate.v1.DetectFlakyTestsResponse.getDefaultInstance()
        stub.detectFlakyTests(_) >> flakyResponse

        def suite = Mock(TestDescriptor)
        suite.getName() >> "root suite"
        suite.getParent() >> null

        def result = Mock(TestResult)
        result.getTestCount() >> 10
        result.getSuccessfulTestCount() >> 8
        result.getFailedTestCount() >> 1
        result.getSkippedTestCount() >> 1

        when:
        listener.afterSuite(suite, result)

        then:
        1 * stub.detectFlakyTests(_)
    }
}
