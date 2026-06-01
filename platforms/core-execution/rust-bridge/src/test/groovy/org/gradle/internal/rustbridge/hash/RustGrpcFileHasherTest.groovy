package org.gradle.internal.rustbridge.hash

import com.google.protobuf.ByteString
import gradle.substrate.v1.HashBatchRequest
import gradle.substrate.v1.HashBatchResponse
import gradle.substrate.v1.HashResult
import gradle.substrate.v1.HashServiceGrpc
import io.grpc.ManagedChannel
import io.grpc.ManagedChannelBuilder
import io.grpc.Server
import io.grpc.netty.shaded.io.grpc.netty.NettyServerBuilder
import io.grpc.stub.StreamObserver
import org.gradle.internal.hash.HashCode
import org.gradle.internal.rustbridge.SubstrateClient
import org.gradle.internal.rustbridge.SubstrateException
import org.gradle.internal.rustbridge.filehashcache.RustFileHashCacheClient
import spock.lang.Specification

import java.nio.file.Files

class RustGrpcFileHasherTest extends Specification {

    private static byte[] hashBytes(int seed) {
        (0..<16).collect { (byte) (seed + it) } as byte[]
    }

    private static HashCode hash(int seed) {
        HashCode.fromBytes(hashBytes(seed))
    }

    private static class RecordingFileHashCacheClient extends RustFileHashCacheClient {
        FileInfoResult nextGet = new FileInfoResult(false, Optional.empty(), "")
        List<Map<String, Object>> puts = []

        RecordingFileHashCacheClient() {
            super(SubstrateClient.noop())
        }

        @Override
        FileInfoResult getFileInfo(String path, long length, long lastModified, String kind) {
            return nextGet
        }

        @Override
        boolean putFileInfo(String path, HashCode hash, long length, long lastModified, String kind) {
            puts.add([path: path, hash: hash, length: length, lastModified: lastModified, kind: kind])
            return true
        }
    }

    private static class TestHashService extends HashServiceGrpc.HashServiceImplBase {
        byte[] responseHash = hashBytes(7)
        int calls

        @Override
        void hashBatch(HashBatchRequest request, StreamObserver<HashBatchResponse> responseObserver) {
            calls++
            responseObserver.onNext(HashBatchResponse.newBuilder()
                .addResults(HashResult.newBuilder()
                    .setAbsolutePath(request.getFiles(0).getAbsolutePath())
                    .setHashBytes(ByteString.copyFrom(responseHash))
                    .setError(false)
                    .build())
                .build())
            responseObserver.onCompleted()
        }
    }

    private static class TestHashHarness implements Closeable {
        final TestHashService service = new TestHashService()
        final Server server
        final ManagedChannel channel

        TestHashHarness() {
            server = NettyServerBuilder
                .forPort(0)
                .addService(service)
                .build()
            server.start()
            channel = ManagedChannelBuilder
                .forAddress("127.0.0.1", server.port)
                .usePlaintext()
                .build()
        }

        HashServiceGrpc.HashServiceBlockingStub blockingStub() {
            HashServiceGrpc.newBlockingStub(channel)
        }

        @Override
        void close() {
            channel.shutdownNow()
            server.shutdownNow()
            server.awaitTermination()
            channel.awaitTermination(5, java.util.concurrent.TimeUnit.SECONDS)
        }
    }

    def "uses file hash cache hit without calling hash service"() {
        given:
        def file = Files.createTempFile("rust-hasher-cache-hit", ".txt").toFile()
        file.text = "cached"
        def cachedHash = hash(42)
        def cache = new RecordingFileHashCacheClient()
        cache.nextGet = new RustFileHashCacheClient.FileInfoResult(
            true,
            Optional.of(new RustFileHashCacheClient.FileInfoData(cachedHash, 6L, 123L)),
            ""
        )
        def client = Mock(SubstrateClient)
        def hasher = new RustGrpcFileHasher(client, cache, false)

        when:
        def result = hasher.hash(file, 6L, 123L)

        then:
        result == cachedHash
        0 * client.getHashStub()
        cache.puts.empty

        cleanup:
        file.delete()
    }

    def "stores file hash cache entry after hash service miss"() {
        given:
        def harness = new TestHashHarness()
        def file = Files.createTempFile("rust-hasher-cache-miss", ".txt").toFile()
        file.text = "uncached"
        def cache = new RecordingFileHashCacheClient()
        def client = Mock(SubstrateClient)
        client.getHashStub() >> harness.blockingStub()
        def hasher = new RustGrpcFileHasher(client, cache, false)

        when:
        def result = hasher.hash(file, 8L, 456L)

        then:
        result == HashCode.fromBytes(harness.service.responseHash)
        harness.service.calls == 1
        cache.puts.size() == 1
        cache.puts[0].path == file.absolutePath
        cache.puts[0].hash == result
        cache.puts[0].length == 8L
        cache.puts[0].lastModified == 456L
        cache.puts[0].kind == "FILE_HASHES"

        cleanup:
        harness.close()
        file.delete()
    }

    def "authoritative file hash cache get error fails before hashing"() {
        given:
        def file = Files.createTempFile("rust-hasher-cache-error", ".txt").toFile()
        file.text = "error"
        def cache = new RecordingFileHashCacheClient()
        cache.nextGet = new RustFileHashCacheClient.FileInfoResult(false, Optional.empty(), "cache unavailable")
        def client = Mock(SubstrateClient)
        def hasher = new RustGrpcFileHasher(client, cache, true)

        when:
        hasher.hash(file, 5L, 789L)

        then:
        thrown(SubstrateException)
        0 * client.getHashStub()

        cleanup:
        file.delete()
    }
}
