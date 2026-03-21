package org.gradle.internal.rustbridge.toolchain;

import gradle.substrate.v1.GetJavaHomeRequest;
import gradle.substrate.v1.GetJavaHomeResponse;
import gradle.substrate.v1.ListToolchainsRequest;
import gradle.substrate.v1.ListToolchainsResponse;
import gradle.substrate.v1.ToolchainServiceGrpc;
import gradle.substrate.v1.VerifyToolchainRequest;
import gradle.substrate.v1.VerifyToolchainResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust toolchain service.
 * Manages JDK/toolchain discovery and verification via gRPC.
 */
public class RustToolchainServiceClient {

    private static final Logger LOGGER = Logging.getLogger(RustToolchainServiceClient.class);

    private final SubstrateClient client;

    public RustToolchainServiceClient(SubstrateClient client) {
        this.client = client;
    }

    public List<ListToolchainsResponse.ToolchainLocation> listToolchains(String os, String arch) {
        if (client.isNoop()) {
            return List.of();
        }

        try {
            ListToolchainsResponse response = client.getToolchainStub()
                .listToolchains(ListToolchainsRequest.newBuilder()
                    .setOs(os)
                    .setArch(arch)
                    .build());
            return response.getToolchainsList();
        } catch (Exception e) {
            LOGGER.debug("[substrate:toolchain] list toolchains failed", e);
            return List.of();
        }
    }

    public VerifyToolchainResponse verifyToolchain(String javaHome, String expectedVersion) {
        if (client.isNoop()) {
            return VerifyToolchainResponse.getDefaultInstance();
        }

        try {
            return client.getToolchainStub()
                .verifyToolchain(VerifyToolchainRequest.newBuilder()
                    .setJavaHome(javaHome)
                    .setExpectedVersion(expectedVersion != null ? expectedVersion : "")
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:toolchain] verify toolchain failed", e);
            return VerifyToolchainResponse.getDefaultInstance();
        }
    }

    public GetJavaHomeResponse getJavaHome(String languageVersion, String implementation) {
        if (client.isNoop()) {
            return GetJavaHomeResponse.getDefaultInstance();
        }

        try {
            return client.getToolchainStub()
                .getJavaHome(GetJavaHomeRequest.newBuilder()
                    .setLanguageVersion(languageVersion)
                    .setImplementation(implementation != null ? implementation : "")
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:toolchain] get java home failed", e);
            return GetJavaHomeResponse.getDefaultInstance();
        }
    }
}
