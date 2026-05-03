/*
 * Copyright 2026 the original author or authors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
package org.gradle.execution

import org.gradle.api.Task
import org.gradle.api.internal.GradleInternal
import org.gradle.execution.plan.FinalizedExecutionPlan
import org.gradle.execution.plan.QueryableExecutionPlan
import org.gradle.internal.build.ExecutionResult
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient
import org.gradle.internal.rustbridge.eventstream.BuildIdHolder
import org.gradle.internal.rustbridge.taskgraph.RustBuildExecutionClient
import spock.lang.Specification

import java.util.function.BiConsumer

class RustAuthoritativeBuildExecutionActionTest extends Specification {
    def delegate = Mock(BuildWorkExecutor)
    def buildExecutionClient = Mock(RustBuildExecutionClient)
    def gradle = Mock(GradleInternal)

    def cleanup() {
        BuildIdHolder.clear()
    }

    def "skips JVM executor when Rust completes the scheduled plan without fallback"() {
        given:
        def action = new RustAuthoritativeBuildExecutionAction(delegate, buildExecutionClient, true)
        def plan = planWithTaskCount(2)

        when:
        def result = action.execute(gradle, plan)

        then:
        1 * buildExecutionClient.runBuild("build", _ as Integer, false) >>
            RustBuildExecutionClient.RunBuildResult.success("build-plan-shadow", 2, 2)
        0 * delegate._
        result.successful
    }

    def "fails closed when Rust reports fewer tasks than Gradle scheduled"() {
        given:
        def action = new RustAuthoritativeBuildExecutionAction(delegate, buildExecutionClient, true)
        def plan = planWithTaskCount(2)

        when:
        def result = action.execute(gradle, plan)

        then:
        1 * buildExecutionClient.runBuild("build", _ as Integer, false) >>
            RustBuildExecutionClient.RunBuildResult.success("build-plan-shadow", 1, 1)
        0 * delegate._
        !result.successful
        result.failure.message.contains("expectedTasks=2")
        result.failure.message.contains("totalTasks=1")
    }

    def "fails closed when Rust needs JVM forwarding"() {
        given:
        def action = new RustAuthoritativeBuildExecutionAction(delegate, buildExecutionClient, true)
        def plan = planWithTaskCount(1)

        when:
        def result = action.execute(gradle, plan)

        then:
        1 * buildExecutionClient.runBuild("build", _ as Integer, false) >>
            RustBuildExecutionClient.RunBuildResult.completed("build-plan-shadow", 1, 1, 0, 1)
        0 * delegate._
        !result.successful
        result.failure.message.contains("jvmForwarded=1")
    }

    def "delegates on incomplete Rust execution when fail closed is disabled"() {
        given:
        def action = new RustAuthoritativeBuildExecutionAction(delegate, buildExecutionClient, false)
        def plan = planWithTaskCount(1)

        when:
        def result = action.execute(gradle, plan)

        then:
        1 * buildExecutionClient.runBuild("build", _ as Integer, false) >>
            RustBuildExecutionClient.RunBuildResult.error("missing native executor")
        1 * delegate.execute(gradle, plan) >> ExecutionResult.succeeded()
        result.successful
    }

    def "uses active substrate build id when available"() {
        given:
        BuildIdHolder.setBuildId("substrate-build-123")
        def action = new RustAuthoritativeBuildExecutionAction(delegate, buildExecutionClient, true)
        def plan = planWithTaskCount(1)

        when:
        action.execute(gradle, plan)

        then:
        1 * buildExecutionClient.runBuild("substrate-build-123", _ as Integer, false) >>
            RustBuildExecutionClient.RunBuildResult.success("build-plan-shadow", 1, 1)
    }

    def "lazily bootstraps scoped build id when lifecycle holder is empty"() {
        given:
        def bootstrapClient = Mock(RustBootstrapClient)
        def action = new RustAuthoritativeBuildExecutionAction(delegate, buildExecutionClient, true, bootstrapClient, null)
        def plan = planWithTaskCount(1)
        String initializedBuildId = null

        when:
        def result = action.execute(gradle, plan)

        then:
        1 * bootstrapClient.initBuild({ it != "build" && !it.empty }, _ as String, _ as Long, _ as Integer, _ as Map, _ as List) >> { args ->
            initializedBuildId = args[0]
            null
        }
        1 * buildExecutionClient.runBuild({ it == initializedBuildId }, _ as Integer, false) >>
            RustBuildExecutionClient.RunBuildResult.success("build-plan-shadow", 1, 1)
        1 * bootstrapClient.completeBuild({ it == initializedBuildId }, "SUCCESS", _ as Long) >> true
        0 * delegate._
        result.successful
        BuildIdHolder.getBuildId() == ""
    }

    private FinalizedExecutionPlan planWithTaskCount(int taskCount) {
        Set<Task> tasks = (1..taskCount).collect { Stub(Task) } as Set
        def scheduledNodes = Stub(QueryableExecutionPlan.ScheduledNodes) {
            visitNodes(_ as BiConsumer) >> { BiConsumer visitor ->
                visitor.accept([], [] as Set)
            }
        }
        def contents = Stub(QueryableExecutionPlan) {
            getTasks() >> tasks
            getScheduledNodes() >> scheduledNodes
        }
        Stub(FinalizedExecutionPlan) {
            getContents() >> contents
        }
    }
}
