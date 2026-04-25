package org.gradle.internal.rustbridge.testexec;

import gradle.substrate.v1.DetectFlakyTestsRequest;
import gradle.substrate.v1.DetectFlakyTestsResponse;
import gradle.substrate.v1.FlakyTestInfo;
import gradle.substrate.v1.RegisterTestSuiteRequest;
import gradle.substrate.v1.ReportTestResultRequest;
import gradle.substrate.v1.TestResultEntry;
import gradle.substrate.v1.TestSuiteDescriptor;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Compile-safe compatibility listener for Gradle test execution events.
 *
 * <p>The runtime Gradle test listener API lives in a Java 17-oriented module, while this
 * bridge still compiles for Java 8. To avoid pinning the bridge to that classpath, the
 * listener handles event payloads reflectively and can expose a dynamic proxy when the
 * runtime test API is present.</p>
 */
public class TestExecutionShadowListener {

    private static final Logger LOGGER = Logging.getLogger(TestExecutionShadowListener.class);
    private static final String TEST_LISTENER_CLASS = "org.gradle.api.tasks.testing.TestListener";

    private final SubstrateClient client;
    private final AtomicInteger suiteCount = new AtomicInteger(0);
    private final AtomicInteger testCount = new AtomicInteger(0);
    private final AtomicInteger passCount = new AtomicInteger(0);
    private final AtomicInteger failCount = new AtomicInteger(0);
    private final AtomicInteger skipCount = new AtomicInteger(0);
    private final List<String> failedTests = new ArrayList<>();

    public TestExecutionShadowListener(SubstrateClient client) {
        this.client = client;
    }

    public Object asListenerProxy() {
        try {
            Class<?> listenerType = Class.forName(TEST_LISTENER_CLASS, false, getClass().getClassLoader());
            InvocationHandler handler = new DispatchingInvocationHandler();
            return Proxy.newProxyInstance(listenerType.getClassLoader(), new Class<?>[] { listenerType }, handler);
        } catch (ClassNotFoundException e) {
            LOGGER.debug("[substrate:testexec] runtime TestListener API unavailable", e);
            return null;
        }
    }

    public void beforeSuite(Object suite) {
        if (client.isNoop()) {
            return;
        }

        try {
            client.getTestExecutionStub().registerTestSuite(
                RegisterTestSuiteRequest.newBuilder()
                    .setBuildId("build")
                    .setSuite(TestSuiteDescriptor.newBuilder()
                        .setSuiteId(defaultSuiteId(suite))
                        .setSuiteName(stringValue(suite, "getName", "suite"))
                        .setSuiteType(stringValue(suite, "getClassName", "").isEmpty() ? "unknown" : "junit")
                        .setTestCount(0)
                        .setModulePath("")
                        .build())
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] shadow beforeSuite failed for {}", stringValue(suite, "getName", "suite"), e);
        }
    }

    public void afterSuite(Object suite, Object result) {
        if (client.isNoop()) {
            return;
        }

        try {
            LOGGER.debug(
                "[substrate:testexec] suite '{}' completed: {} passed, {} failed, {} skipped",
                stringValue(suite, "getName", "suite"),
                longValue(result, "getSuccessfulTestCount"),
                longValue(result, "getFailedTestCount"),
                longValue(result, "getSkippedTestCount")
            );

            if (objectValue(suite, "getParent") == null && longValue(result, "getTestCount") > 0) {
                DetectFlakyTestsResponse flakyResponse = client.getTestExecutionStub()
                    .detectFlakyTests(
                        DetectFlakyTestsRequest.newBuilder()
                            .setBuildId("build")
                            .build()
                    );
                if (flakyResponse.getFlakyTestsCount() > 0) {
                    LOGGER.warn("[substrate:testexec] {} flaky test(s) detected:", flakyResponse.getFlakyTestsCount());
                    for (FlakyTestInfo flaky : flakyResponse.getFlakyTestsList()) {
                        LOGGER.warn(
                            "  - {} > {} (flake rate: {}%, {}/{} runs failed)",
                            flaky.getTestClass(),
                            flaky.getTestName(),
                            String.format("%.1f", flaky.getFlakeRate() * 100),
                            flaky.getFailCount(),
                            flaky.getPassCount() + flaky.getFailCount()
                        );
                    }
                }
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] shadow afterSuite failed", e);
        }
    }

    public void beforeTest(Object testDescriptor) {
        // Nothing to do before individual test
    }

    public void afterTest(Object testDescriptor, Object result) {
        if (client.isNoop()) {
            return;
        }

        try {
            testCount.incrementAndGet();

            String outcome = stringValue(objectValue(result, "getResultType"), "toString", "SKIPPED");
            if ("SUCCESS".equals(outcome)) {
                passCount.incrementAndGet();
                outcome = "PASSED";
            } else if ("FAILURE".equals(outcome)) {
                failCount.incrementAndGet();
                failedTests.add(stringValue(testDescriptor, "getName", "unknown"));
                outcome = "FAILED";
            } else {
                skipCount.incrementAndGet();
                outcome = "SKIPPED";
            }

            String suiteId = defaultSuiteId(testDescriptor);
            long startTime = longValue(result, "getStartTime");
            long endTime = longValue(result, "getEndTime");
            Throwable failure = throwableValue(result, "getException");

            client.getTestExecutionStub().reportTestResult(
                ReportTestResultRequest.newBuilder()
                    .setBuildId("build")
                    .setResult(TestResultEntry.newBuilder()
                        .setTestId(suiteId + "." + stringValue(testDescriptor, "getName", "unknown"))
                        .setSuiteId(suiteId)
                        .setTestName(stringValue(testDescriptor, "getName", "unknown"))
                        .setTestClass(suiteId)
                        .setOutcome(outcome)
                        .setStartTimeMs(startTime)
                        .setEndTimeMs(endTime)
                        .setDurationMs(endTime - startTime)
                        .setFailureMessage(failure != null && failure.getMessage() != null ? failure.getMessage() : "")
                        .setFailureType(failure != null ? failure.getClass().getSimpleName() : "")
                        .build())
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug(
                "[substrate:testexec] shadow afterTest failed for {}",
                stringValue(testDescriptor, "getName", "unknown"),
                e
            );
        }
    }

    private String defaultSuiteId(Object descriptor) {
        String className = stringValue(descriptor, "getClassName", "");
        return className.isEmpty() ? "suite-" + suiteCount.incrementAndGet() : className;
    }

    private static String stringValue(Object target, String methodName, String defaultValue) {
        Object value = objectValue(target, methodName);
        return value == null ? defaultValue : String.valueOf(value);
    }

    private static long longValue(Object target, String methodName) {
        Object value = objectValue(target, methodName);
        return value instanceof Number ? ((Number) value).longValue() : 0L;
    }

    private static Throwable throwableValue(Object target, String methodName) {
        Object value = objectValue(target, methodName);
        return value instanceof Throwable ? (Throwable) value : null;
    }

    private static Object objectValue(Object target, String methodName) {
        if (target == null) {
            return null;
        }
        try {
            Method method = target.getClass().getMethod(methodName);
            return method.invoke(target);
        } catch (NoSuchMethodException e) {
            return null;
        } catch (IllegalAccessException e) {
            return null;
        } catch (InvocationTargetException e) {
            return null;
        }
    }

    private class DispatchingInvocationHandler implements InvocationHandler {
        @Override
        public Object invoke(Object proxy, Method method, Object[] args) {
            String methodName = method.getName();
            if ("beforeSuite".equals(methodName) && args != null && args.length == 1) {
                beforeSuite(args[0]);
            } else if ("afterSuite".equals(methodName) && args != null && args.length == 2) {
                afterSuite(args[0], args[1]);
            } else if ("beforeTest".equals(methodName) && args != null && args.length == 1) {
                beforeTest(args[0]);
            } else if ("afterTest".equals(methodName) && args != null && args.length == 2) {
                afterTest(args[0], args[1]);
            } else if ("toString".equals(methodName)) {
                return "TestExecutionShadowListenerProxy";
            }
            return null;
        }
    }
}
