package org.gradle.internal.rustbridge.problems;

import gradle.substrate.v1.ClearProblemsRequest;
import gradle.substrate.v1.ClearProblemsResponse;
import gradle.substrate.v1.GetProblemsBySeverityRequest;
import gradle.substrate.v1.GetProblemsRequest;
import gradle.substrate.v1.GetProblemsResponse;
import gradle.substrate.v1.ProblemDetails;
import gradle.substrate.v1.ProblemReportingServiceGrpc;
import gradle.substrate.v1.ReportProblemRequest;
import gradle.substrate.v1.ReportProblemResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust problem reporting service.
 * Reports and queries build problems/diagnostics via gRPC.
 */
public class RustProblemReportingClient {

    private static final Logger LOGGER = Logging.getLogger(RustProblemReportingClient.class);

    private final SubstrateClient client;

    public RustProblemReportingClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean reportProblem(String buildId, String severity, String category,
                                  String message, String details, String filePath,
                                  int lineNumber, int column, String contextualLabel,
                                  String documentationUrl) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ProblemDetails problem = ProblemDetails.newBuilder()
                .setSeverity(severity)
                .setCategory(category)
                .setMessage(message)
                .setDetails(details != null ? details : "")
                .setFilePath(filePath != null ? filePath : "")
                .setLineNumber(lineNumber)
                .setColumn(column)
                .setContextualLabel(contextualLabel != null ? contextualLabel : "")
                .setDocumentationUrl(documentationUrl != null ? documentationUrl : "")
                .setTimestampMs(System.currentTimeMillis())
                .build();

            ReportProblemResponse response = client.getProblemReportingStub()
                .reportProblem(ReportProblemRequest.newBuilder()
                    .setBuildId(buildId)
                    .setProblem(problem)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:problems] report problem failed", e);
            return false;
        }
    }

    public GetProblemsResponse getProblems(String buildId) {
        if (client.isNoop()) {
            return GetProblemsResponse.getDefaultInstance();
        }

        try {
            return client.getProblemReportingStub()
                .getProblems(GetProblemsRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:problems] get problems failed", e);
            return GetProblemsResponse.getDefaultInstance();
        }
    }

    public GetProblemsResponse getProblemsBySeverity(String buildId, String severity) {
        if (client.isNoop()) {
            return GetProblemsResponse.getDefaultInstance();
        }

        try {
            return client.getProblemReportingStub()
                .getProblemsBySeverity(GetProblemsBySeverityRequest.newBuilder()
                    .setBuildId(buildId)
                    .setSeverity(severity)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:problems] get problems by severity failed", e);
            return GetProblemsResponse.getDefaultInstance();
        }
    }

    public int clearProblems(String buildId) {
        if (client.isNoop()) {
            return 0;
        }

        try {
            ClearProblemsResponse response = client.getProblemReportingStub()
                .clearProblems(ClearProblemsRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
            return response.getCleared();
        } catch (Exception e) {
            LOGGER.debug("[substrate:problems] clear problems failed", e);
            return 0;
        }
    }
}
