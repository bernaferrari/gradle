package org.gradle.internal.rustbridge.toolchain;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;
import org.jspecify.annotations.Nullable;

import java.util.List;

/**
 * Shadow adapter that compares JVM toolchain resolution results with Rust.
 *
 * <p>In shadow mode, runs both JVM and Rust toolchain lookups and reports mismatches.
 * In authoritative mode, uses the Rust result as primary.</p>
 */
public class ShadowingToolchainProvider {

    private static final Logger LOGGER = Logging.getLogger(ShadowingToolchainProvider.class);

    private final RustToolchainServiceClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingToolchainProvider(
        RustToolchainServiceClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this(rustClient, mismatchReporter, false);
    }

    public ShadowingToolchainProvider(
        RustToolchainServiceClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    /**
     * Compare JVM toolchain list with Rust toolchain list.
     */
    public void compareToolchainLists(String os, String arch,
                                       List<String> javaHomes) {
        try {
            List<?> rustToolchains = rustClient.listToolchains(os, arch);
            int rustCount = rustToolchains.size();
            int javaCount = javaHomes.size();

            if (rustCount != javaCount) {
                mismatchReporter.reportMismatch(
                    "toolchain-list:" + os + "/" + arch,
                    String.valueOf(javaCount),
                    String.valueOf(rustCount)
                );
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("toolchain-list:" + os + "/" + arch, e);
        }
    }

    /**
     * Compare JVM toolchain verification with Rust verification.
     */
    public void compareToolchainVerification(String javaHome, String expectedVersion,
                                              boolean javaValid, @Nullable String javaError) {
        try {
            gradle.substrate.v1.VerifyToolchainResponse rustResponse =
                rustClient.verifyToolchain(javaHome, expectedVersion);
            boolean rustValid = rustResponse.getValid();

            if (javaValid != rustValid) {
                mismatchReporter.reportMismatch(
                    "toolchain-verify:" + javaHome,
                    String.valueOf(javaValid),
                    String.valueOf(rustValid)
                );
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("toolchain-verify:" + javaHome, e);
        }
    }

    /**
     * Compare JVM java-home lookup with Rust java-home lookup.
     */
    public void compareJavaHomeLookup(String languageVersion, String implementation,
                                       @Nullable String javaResult) {
        try {
            gradle.substrate.v1.GetJavaHomeResponse rustResponse =
                rustClient.getJavaHome(languageVersion, implementation);
            String rustResult = rustResponse.getJavaHome();

            if (javaResult == null && rustResult.isEmpty()) {
                mismatchReporter.reportMatch();
            } else if (javaResult == null || !javaResult.equals(rustResult)) {
                mismatchReporter.reportMismatch(
                    "java-home:" + languageVersion + ":" + implementation,
                    javaResult != null ? javaResult : "(null)",
                    rustResult.isEmpty() ? "(empty)" : rustResult
                );
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("java-home:" + languageVersion, e);
        }
    }

    public boolean isAuthoritative() {
        return authoritative;
    }
}
