package org.gradle.internal.rustbridge.incremental;

import gradle.substrate.v1.CompilationUnit;
import gradle.substrate.v1.GetIncrementalStateRequest;
import gradle.substrate.v1.GetIncrementalStateResponse;
import gradle.substrate.v1.GetRebuildSetRequest;
import gradle.substrate.v1.GetRebuildSetResponse;
import gradle.substrate.v1.IncrementalCompilationServiceGrpc;
import gradle.substrate.v1.RecordCompilationRequest;
import gradle.substrate.v1.RecordCompilationResponse;
import gradle.substrate.v1.RegisterSourceSetRequest;
import gradle.substrate.v1.RegisterSourceSetResponse;
import gradle.substrate.v1.SourceSetDescriptor;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust incremental compilation service.
 * Tracks source sets, compilation units, and rebuild decisions via gRPC.
 */
public class RustIncrementalCompilationClient {

    private static final Logger LOGGER = Logging.getLogger(RustIncrementalCompilationClient.class);

    private final SubstrateClient client;

    public RustIncrementalCompilationClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean registerSourceSet(String buildId, String sourceSetId, String name,
                                      List<String> sourceDirs, List<String> outputDirs,
                                      String classpathHash) {
        if (client.isNoop()) {
            return false;
        }

        try {
            SourceSetDescriptor descriptor = SourceSetDescriptor.newBuilder()
                .setSourceSetId(sourceSetId)
                .setName(name)
                .addAllSourceDirs(sourceDirs)
                .addAllOutputDirs(outputDirs)
                .setClasspathHash(classpathHash != null ? classpathHash : "")
                .build();

            RegisterSourceSetResponse response = client.getIncrementalCompilationStub()
                .registerSourceSet(RegisterSourceSetRequest.newBuilder()
                    .setBuildId(buildId)
                    .setSourceSet(descriptor)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental] register source set failed for {}", sourceSetId, e);
            return false;
        }
    }

    public boolean recordCompilation(String buildId, String sourceSetId, String sourceFile,
                                      String outputClass, String sourceHash, String classHash,
                                      List<String> dependencies, long compileDurationMs) {
        if (client.isNoop()) {
            return false;
        }

        try {
            CompilationUnit unit = CompilationUnit.newBuilder()
                .setSourceSetId(sourceSetId)
                .setSourceFile(sourceFile)
                .setOutputClass(outputClass)
                .setSourceHash(sourceHash)
                .setClassHash(classHash)
                .addAllDependencies(dependencies)
                .setCompileDurationMs(compileDurationMs)
                .build();

            RecordCompilationResponse response = client.getIncrementalCompilationStub()
                .recordCompilation(RecordCompilationRequest.newBuilder()
                    .setBuildId(buildId)
                    .setUnit(unit)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental] record compilation failed", e);
            return false;
        }
    }

    public GetRebuildSetResponse getRebuildSet(String buildId, String sourceSetId,
                                                List<String> changedFiles) {
        if (client.isNoop()) {
            return GetRebuildSetResponse.getDefaultInstance();
        }

        try {
            return client.getIncrementalCompilationStub()
                .getRebuildSet(GetRebuildSetRequest.newBuilder()
                    .setBuildId(buildId)
                    .setSourceSetId(sourceSetId)
                    .addAllChangedFiles(changedFiles)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental] get rebuild set failed", e);
            return GetRebuildSetResponse.getDefaultInstance();
        }
    }

    public GetIncrementalStateResponse getIncrementalState(String buildId, String sourceSetId) {
        if (client.isNoop()) {
            return GetIncrementalStateResponse.getDefaultInstance();
        }

        try {
            return client.getIncrementalCompilationStub()
                .getIncrementalState(GetIncrementalStateRequest.newBuilder()
                    .setBuildId(buildId)
                    .setSourceSetId(sourceSetId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:incremental] get incremental state failed", e);
            return GetIncrementalStateResponse.getDefaultInstance();
        }
    }
}
