package org.gradle.internal.rustbridge.cache;

import gradle.substrate.v1.CacheLoadChunk;
import gradle.substrate.v1.CacheLoadRequest;
import gradle.substrate.v1.CacheStoreChunk;
import gradle.substrate.v1.CacheStoreInit;
import gradle.substrate.v1.CacheStoreResponse;
import io.grpc.stub.StreamObserver;
import org.gradle.caching.BuildCacheEntryReader;
import org.gradle.caching.BuildCacheEntryWriter;
import org.gradle.caching.BuildCacheException;
import org.gradle.caching.BuildCacheKey;
import org.gradle.caching.BuildCacheService;
import org.gradle.internal.rustbridge.SubstrateClient;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.Iterator;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/**
 * A {@link BuildCacheService} that delegates to the Rust substrate daemon's
 * remote cache via gRPC. The Rust daemon handles remote GET/PUT internally,
 * including retry logic and authentication.
 *
 * <p>This is distinct from {@link RustBuildCacheService} which handles the local
 * filesystem cache. This service uses the same CacheService gRPC interface but
 * the daemon is configured with a remote cache URL, so it tries remote first
 * then falls back to local.</p>
 */
public class RustRemoteBuildCacheService implements BuildCacheService {

    private final SubstrateClient client;

    public RustRemoteBuildCacheService(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public boolean load(BuildCacheKey key, BuildCacheEntryReader reader) throws BuildCacheException {
        if (client.isNoop()) {
            return false;
        }

        try {
            // Server-streaming RPC via blocking stub returns an iterator
            Iterator<CacheLoadChunk> chunks = client.getCacheStub().loadEntry(
                CacheLoadRequest.newBuilder()
                    .setKey(com.google.protobuf.ByteString.copyFrom(key.toByteArray()))
                    .build()
            );

            boolean found = false;
            ByteArrayOutputStream collected = new ByteArrayOutputStream();

            while (chunks.hasNext()) {
                CacheLoadChunk chunk = chunks.next();
                if (chunk.hasData()) {
                    collected.write(chunk.getData().toByteArray());
                    found = true;
                }
            }

            if (!found) {
                return false;
            }

            reader.readFrom(new ByteArrayInputStream(collected.toByteArray()));
            return true;
        } catch (IOException e) {
            throw new BuildCacheException("Failed to read from Rust remote cache", e);
        } catch (Exception e) {
            throw new BuildCacheException("Failed to load from Rust remote cache", e);
        }
    }

    @Override
    public void store(BuildCacheKey key, BuildCacheEntryWriter writer) throws BuildCacheException {
        if (client.isNoop()) {
            return;
        }

        try {
            // Capture the entry bytes
            ByteArrayOutputStream baos = new ByteArrayOutputStream();
            writer.writeTo(baos);
            byte[] data = baos.toByteArray();

            // Client-streaming RPC via async stub: send Init chunk + Data chunk
            AtomicBoolean success = new AtomicBoolean(false);
            AtomicReference<String> errorMessage = new AtomicReference<>("");
            CountDownLatch latch = new CountDownLatch(1);

            StreamObserver<CacheStoreChunk> requestObserver =
                client.getCacheAsyncStub().storeEntry(new StreamObserver<CacheStoreResponse>() {
                    @Override
                    public void onNext(CacheStoreResponse response) {
                        success.set(response.getSuccess());
                        errorMessage.set(response.getErrorMessage());
                    }

                    @Override
                    public void onError(Throwable t) {
                        errorMessage.set(t.getMessage());
                        latch.countDown();
                    }

                    @Override
                    public void onCompleted() {
                        latch.countDown();
                    }
                });

            // Send Init chunk with metadata
            requestObserver.onNext(CacheStoreChunk.newBuilder()
                .setInit(CacheStoreInit.newBuilder()
                    .setKey(com.google.protobuf.ByteString.copyFrom(key.toByteArray()))
                    .setTotalSize(data.length)
                    .build())
                .build());

            // Send Data chunk with actual content
            requestObserver.onNext(CacheStoreChunk.newBuilder()
                .setData(com.google.protobuf.ByteString.copyFrom(data))
                .build());

            // Signal completion of the stream
            requestObserver.onCompleted();

            // Wait for the server response
            latch.await();

            if (!success.get() && !errorMessage.get().isEmpty()) {
                throw new BuildCacheException("Failed to store in Rust remote cache: " + errorMessage.get());
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new BuildCacheException("Interrupted while storing to Rust remote cache", e);
        } catch (IOException e) {
            throw new BuildCacheException("Failed to write to Rust remote cache", e);
        }
    }

    @Override
    public void close() throws IOException {
        // No-op; client is shared
    }
}
