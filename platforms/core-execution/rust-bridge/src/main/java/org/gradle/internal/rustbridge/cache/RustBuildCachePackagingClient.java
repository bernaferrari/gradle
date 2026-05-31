package org.gradle.internal.rustbridge.cache;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

/**
 * Compile-safe facade for the build-cache packaging slice.
 *
 * <p>The Java bridge previously referenced protobuf messages and a gRPC service
 * that are not present in the generated schema. Until that contract exists,
 * this client must stay fail-closed and observable instead of linking against
 * missing generated classes.</p>
 */
public class RustBuildCachePackagingClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildCachePackagingClient.class);

    private final SubstrateClient client;

    public RustBuildCachePackagingClient(SubstrateClient client) {
        this.client = client;
    }

    public PackResult packCacheEntry(String buildId, Object spec) {
        if (!client.isNoop()) {
            LOGGER.debug("[substrate:cache-packaging] protobuf contract is not available; skipping pack for {}", buildId);
        }
        return PackResult.unsupported("cache-packaging-proto-unavailable");
    }

    public PackResult unpackCacheEntry(String buildId, byte[] packagedBytes) {
        if (!client.isNoop()) {
            LOGGER.debug("[substrate:cache-packaging] protobuf contract is not available; skipping unpack for {}", buildId);
        }
        return PackResult.unsupported("cache-packaging-proto-unavailable");
    }

    public static final class PackResult {
        private final boolean success;
        private final byte[] packagedBytes;
        private final String error;

        private PackResult(boolean success, byte[] packagedBytes, String error) {
            this.success = success;
            this.packagedBytes = packagedBytes == null ? new byte[0] : packagedBytes;
            this.error = error == null ? "" : error;
        }

        public static PackResult unsupported(String error) {
            return new PackResult(false, new byte[0], error);
        }

        public boolean isSuccess() {
            return success;
        }

        public byte[] getPackagedBytes() {
            return packagedBytes;
        }

        public String getError() {
            return error;
        }
    }
}
