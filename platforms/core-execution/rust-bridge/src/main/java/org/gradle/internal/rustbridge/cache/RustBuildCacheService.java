package org.gradle.internal.rustbridge.cache;

import gradle.substrate.v1.CacheServiceGrpc;
import gradle.substrate.v1.CacheStoreChunk;
import gradle.substrate.v1.CacheStoreInit;
import io.grpc.stub.StreamObserver;
import org.gradle.caching.BuildCacheEntryReader;
import org.gradle.caching.BuildCacheEntryWriter;
import org.gradle.caching.BuildCacheException;
import org.gradle.caching.BuildCacheKey;
import org.gradle.caching.BuildCacheService;
import org.gradle.internal.rustbridge.SubstrateClient;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/**
 * A {@link BuildCacheService} implementation that delegates storage operations
 * to the Rust substrate daemon via gRPC.
 */
public class RustBuildCacheService implements BuildCacheService {

    private final SubstrateClient client;
    private final String description;

    public RustBuildCacheService(SubstrateClient client, String description) {
        this.client = client;
        this.description = description;
    }

    @Override
    public boolean load(BuildCacheKey key, BuildCacheEntryReader reader) throws BuildCacheException {
        if (client.isNoop()) {
            return false;
        }

        try {
            AtomicBoolean found = new AtomicBoolean(false);
            AtomicReference<byte[]> dataRef = new AtomicReference<>();
            CountDownLatch latch = new CountDownLatch(1);

            client.getCacheStub().loadEntry(
                gradle.substrate.v1.CacheLoadRequest.newBuilder()
                    .setKey(com.google.protobuf.ByteString.copyFrom(key.toByteArray()))
                    .build(),
                new StreamObserver<gradle.substrate.v1.CacheLoadChunk>() {
                    @Override
                    public void onNext(gradle.substrate.v1.CacheLoadChunk chunk) {
                        if (chunk.hasData()) {
                            byte[] existing = dataRef.get();
                            byte[] newData = chunk.getData().toByteArray();
                            if (existing == null) {
                                dataRef.set(newData);
                            } else {
                                byte[] combined = new byte[existing.length + newData.length];
                                System.arraycopy(existing, 0, combined, 0, existing.length);
                                System.arraycopy(newData, 0, combined, existing.length, newData.length);
                                dataRef.set(combined);
                            }
                            found.set(true);
                        }
                    }

                    @Override
                    public void onError(Throwable t) {
                        latch.countDown();
                    }

                    @Override
                    public void onCompleted() {
                        latch.countDown();
                    }
                }
            );

            latch.await();

            if (!found.get()) {
                return false;
            }

            byte[] data = dataRef.get();
            if (data == null) {
                return false;
            }

            reader.readFrom(new java.io.ByteArrayInputStream(data));
            return true;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new BuildCacheException("Interrupted while loading from Rust cache", e);
        } catch (IOException e) {
            throw new BuildCacheException("Failed to read from Rust cache", e);
        }
    }

    @Override
    public void store(BuildCacheKey key, BuildCacheEntryWriter writer) throws BuildCacheException {
        if (client.isNoop()) {
            return;
        }

        try {
            // First capture the entry bytes (writer writes to an OutputStream)
            java.io.ByteArrayOutputStream baos = new java.io.ByteArrayOutputStream();
            writer.writeTo(baos);
            byte[] data = baos.toByteArray();

            // Stream to Rust via gRPC
            gradle.substrate.v1.CacheStoreResponse response = client.getCacheStub()
                .storeEntry(CacheStoreChunk.newBuilder()
                    .setInit(CacheStoreInit.newBuilder()
                        .setKey(com.google.protobuf.ByteString.copyFrom(key.toByteArray()))
                        .setTotalSize(data.length)
                        .build())
                    .build())
                .block();

            if (!response.getSuccess()) {
                throw new BuildCacheException("Failed to store in Rust cache: " + response.getErrorMessage());
            }
        } catch (IOException e) {
            throw new BuildCacheException("Failed to write to Rust cache", e);
        }
    }

    @Override
    public void close() throws IOException {
        // No-op; client is shared
    }
}
