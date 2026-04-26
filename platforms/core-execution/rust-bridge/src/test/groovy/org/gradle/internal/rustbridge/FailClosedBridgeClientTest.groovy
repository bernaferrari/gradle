package org.gradle.internal.rustbridge

import org.gradle.internal.rustbridge.buildinit.RustBuildInitClient
import org.gradle.internal.rustbridge.buildlayout.RustBuildLayoutClient
import org.gradle.internal.rustbridge.buildops.RustBuildOperationsClient
import org.gradle.internal.rustbridge.buildresult.RustBuildResultClient
import org.gradle.internal.rustbridge.problems.RustProblemReportingClient
import org.gradle.internal.rustbridge.testexec.RustTestExecutionClient
import org.gradle.internal.rustbridge.worker.RustWorkerProcessClient
import spock.lang.Specification

class FailClosedBridgeClientTest extends Specification {

    def "hardened bridge clients throw on no-op substrate instead of returning defaults"() {
        given:
        def substrate = SubstrateClient.noop("missing daemon")

        expect:
        failsClosed { new RustBuildResultClient(substrate).getBuildResult("build") }
        failsClosed { new RustBuildLayoutClient(substrate).listProjects("build") }
        failsClosed { new RustBuildInitClient(substrate).getBuildInitStatus("build") }
        failsClosed { new RustBuildOperationsClient(substrate).getBuildSummary() }
        failsClosed { new RustProblemReportingClient(substrate).getProblems("build") }
        failsClosed { new RustTestExecutionClient(substrate).getTestSummary("build") }
        failsClosed {
            new RustWorkerProcessClient(substrate).getWorkerStatus("compileJava")
        }
    }

    private static void failsClosed(Closure<?> action) {
        try {
            action.call()
            throw new AssertionError("Expected SubstrateException")
        } catch (SubstrateException e) {
            assert e.message.contains("missing daemon")
        }
    }
}
