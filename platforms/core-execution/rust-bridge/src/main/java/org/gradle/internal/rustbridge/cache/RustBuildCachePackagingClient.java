package org.gradle.internal.rustbridge.cache;

import com.google.protobuf.ByteString;
import gradle.substrate.v1.BuildCachePackFile;
import gradle.substrate.v1.PackCacheEntryRequest;
import gradle.substrate.v1.PackCacheEntryResponse;
import gradle.substrate.v1.UnpackCacheEntryRequest;
import gradle.substrate.v1.UnpackCacheEntryResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Thin client for the Rust build-cache packaging service.
 */
public class RustBuildCachePackagingClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildCachePackagingClient.class);

    private final SubstrateClient client;

    public RustBuildCachePackagingClient(SubstrateClient client) {
        this.client = client;
    }

    public PackResult packCacheEntry(String buildId, Object spec) {
        if (client.isNoop()) {
            return PackResult.unsupported("substrate-disabled");
        }
        PackSpec packSpec = PackSpec.from(spec);
        if (packSpec == null) {
            return PackResult.unsupported("unsupported-pack-spec:" + (spec == null ? "null" : spec.getClass().getName()));
        }

        try {
            PackCacheEntryRequest.Builder request = PackCacheEntryRequest.newBuilder()
                .setBuildId(buildId == null ? "" : buildId)
                .setGzip(packSpec.isGzip())
                .putAllOriginMetadata(packSpec.getOriginMetadata());
            for (Entry entry : packSpec.getEntries()) {
                request.addFiles(BuildCachePackFile.newBuilder()
                    .setPath(entry.getPath())
                    .setContent(ByteString.copyFrom(entry.getContent()))
                    .setExecutable(entry.isExecutable())
                    .build());
            }
            PackCacheEntryResponse response = client.getCachePackagingStub().packCacheEntry(request.build());
            if (!response.getSuccess()) {
                return PackResult.unsupported(response.getError());
            }
            return PackResult.packed(response.getPackagedBytes().toByteArray(), response.getEntryCount());
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate:cache-packaging] pack failed for {}", buildId, e);
            return PackResult.unsupported(e.getMessage());
        }
    }

    public PackResult unpackCacheEntry(String buildId, byte[] packagedBytes) {
        return unpackCacheEntry(buildId, packagedBytes, true);
    }

    public PackResult unpackCacheEntry(String buildId, byte[] packagedBytes, boolean gzip) {
        if (client.isNoop()) {
            return PackResult.unsupported("substrate-disabled");
        }
        try {
            UnpackCacheEntryResponse response = client.getCachePackagingStub().unpackCacheEntry(
                UnpackCacheEntryRequest.newBuilder()
                    .setBuildId(buildId == null ? "" : buildId)
                    .setPackagedBytes(ByteString.copyFrom(packagedBytes == null ? new byte[0] : packagedBytes))
                    .setGzip(gzip)
                    .build()
            );
            if (!response.getSuccess()) {
                return PackResult.unsupported(response.getError());
            }
            List<Entry> entries = new ArrayList<Entry>(response.getFilesCount());
            for (BuildCachePackFile file : response.getFilesList()) {
                entries.add(new Entry(file.getPath(), file.getContent().toByteArray(), file.getExecutable()));
            }
            return PackResult.unpacked(entries, response.getOriginMetadataMap(), response.getEntryCount());
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate:cache-packaging] unpack failed for {}", buildId, e);
            return PackResult.unsupported(e.getMessage());
        }
    }

    public static final class PackSpec {
        private final List<Entry> entries;
        private final Map<String, String> originMetadata;
        private final boolean gzip;

        public PackSpec(List<Entry> entries, Map<String, String> originMetadata, boolean gzip) {
            this.entries = Collections.unmodifiableList(new ArrayList<Entry>(entries == null ? Collections.<Entry>emptyList() : entries));
            this.originMetadata = Collections.unmodifiableMap(new LinkedHashMap<String, String>(
                originMetadata == null ? Collections.<String, String>emptyMap() : originMetadata
            ));
            this.gzip = gzip;
        }

        public static PackSpec of(List<Entry> entries, Map<String, String> originMetadata) {
            return new PackSpec(entries, originMetadata, true);
        }

        public static PackSpec singleFile(String path, byte[] content) {
            return new PackSpec(
                Collections.singletonList(new Entry(path, content, false)),
                Collections.<String, String>emptyMap(),
                true
            );
        }

        private static PackSpec from(Object spec) {
            if (spec instanceof PackSpec) {
                return (PackSpec) spec;
            }
            if (spec instanceof byte[]) {
                return singleFile("entry.bin", (byte[]) spec);
            }
            if (spec instanceof String) {
                return singleFile("entry.txt", ((String) spec).getBytes(java.nio.charset.StandardCharsets.UTF_8));
            }
            return null;
        }

        public List<Entry> getEntries() {
            return entries;
        }

        public Map<String, String> getOriginMetadata() {
            return originMetadata;
        }

        public boolean isGzip() {
            return gzip;
        }
    }

    public static final class Entry {
        private final String path;
        private final byte[] content;
        private final boolean executable;

        public Entry(String path, byte[] content, boolean executable) {
            this.path = path == null ? "" : path;
            this.content = content == null ? new byte[0] : content.clone();
            this.executable = executable;
        }

        public String getPath() {
            return path;
        }

        public byte[] getContent() {
            return content.clone();
        }

        public boolean isExecutable() {
            return executable;
        }
    }

    public static final class PackResult {
        private final boolean success;
        private final byte[] packagedBytes;
        private final List<Entry> entries;
        private final Map<String, String> originMetadata;
        private final long entryCount;
        private final String error;

        private PackResult(
            boolean success,
            byte[] packagedBytes,
            List<Entry> entries,
            Map<String, String> originMetadata,
            long entryCount,
            String error
        ) {
            this.success = success;
            this.packagedBytes = packagedBytes == null ? new byte[0] : packagedBytes.clone();
            this.entries = Collections.unmodifiableList(new ArrayList<Entry>(entries == null ? Collections.<Entry>emptyList() : entries));
            this.originMetadata = Collections.unmodifiableMap(new LinkedHashMap<String, String>(
                originMetadata == null ? Collections.<String, String>emptyMap() : originMetadata
            ));
            this.entryCount = entryCount;
            this.error = error == null ? "" : error;
        }

        public static PackResult packed(byte[] packagedBytes, long entryCount) {
            return new PackResult(true, packagedBytes, Collections.<Entry>emptyList(), Collections.<String, String>emptyMap(), entryCount, "");
        }

        public static PackResult unpacked(List<Entry> entries, Map<String, String> originMetadata, long entryCount) {
            return new PackResult(true, new byte[0], entries, originMetadata, entryCount, "");
        }

        public static PackResult unsupported(String error) {
            return new PackResult(false, new byte[0], Collections.<Entry>emptyList(), Collections.<String, String>emptyMap(), 0, error);
        }

        public boolean isSuccess() {
            return success;
        }

        public byte[] getPackagedBytes() {
            return packagedBytes.clone();
        }

        public List<Entry> getEntries() {
            return entries;
        }

        public Map<String, String> getOriginMetadata() {
            return originMetadata;
        }

        public long getEntryCount() {
            return entryCount;
        }

        public String getError() {
            return error;
        }
    }
}
