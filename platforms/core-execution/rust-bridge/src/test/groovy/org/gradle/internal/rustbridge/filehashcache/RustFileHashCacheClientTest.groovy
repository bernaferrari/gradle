package org.gradle.internal.rustbridge.filehashcache

import com.google.protobuf.ByteString
import gradle.substrate.v1.FileHashCacheServiceGrpc
import gradle.substrate.v1.FileInfo
import gradle.substrate.v1.GetFileHashCacheStatsRequest
import gradle.substrate.v1.GetFileHashCacheStatsResponse
import gradle.substrate.v1.GetFileInfoRequest
import gradle.substrate.v1.GetFileInfoResponse
import gradle.substrate.v1.InvalidateFileInfoRequest
import gradle.substrate.v1.InvalidateFileInfoResponse
import gradle.substrate.v1.PutFileInfoRequest
import gradle.substrate.v1.PutFileInfoResponse
import io.grpc.ManagedChannel
import io.grpc.ManagedChannelBuilder
import io.grpc.Server
import io.grpc.netty.shaded.io.grpc.netty.NettyServerBuilder
import io.grpc.stub.StreamObserver
import org.gradle.internal.hash.HashCode
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

import java.nio.file.Files

class RustFileHashCacheClientTest extends Specification {
    private static class TestFileHashCacheService extends FileHashCacheServiceGrpc.FileHashCacheServiceImplBase {
        volatile GetFileInfoRequest lastGet
        volatile PutFileInfoRequest lastPut
        volatile InvalidateFileInfoRequest lastInvalidate
        GetFileInfoResponse getResponse = GetFileInfoResponse.newBuilder().build()
        PutFileInfoResponse putResponse = PutFileInfoResponse.newBuilder().setSuccess(true).build()
        InvalidateFileInfoResponse invalidateResponse = InvalidateFileInfoResponse.newBuilder().setSuccess(true).build()
        GetFileHashCacheStatsResponse statsResponse = GetFileHashCacheStatsResponse.newBuilder().build()

        @Override
        void getFileInfo(GetFileInfoRequest request, StreamObserver<GetFileInfoResponse> responseObserver) {
            lastGet = request
            responseObserver.onNext(getResponse)
            responseObserver.onCompleted()
        }

        @Override
        void putFileInfo(PutFileInfoRequest request, StreamObserver<PutFileInfoResponse> responseObserver) {
            lastPut = request
            responseObserver.onNext(putResponse)
            responseObserver.onCompleted()
        }

        @Override
        void invalidateFileInfo(InvalidateFileInfoRequest request, StreamObserver<InvalidateFileInfoResponse> responseObserver) {
            lastInvalidate = request
            responseObserver.onNext(invalidateResponse)
            responseObserver.onCompleted()
        }

        @Override
        void getStats(GetFileHashCacheStatsRequest request, StreamObserver<GetFileHashCacheStatsResponse> responseObserver) {
            responseObserver.onNext(statsResponse)
            responseObserver.onCompleted()
        }
    }

    private static class TestFileHashCacheHarness implements Closeable {
        final File tempDir
        final TestFileHashCacheService service = new TestFileHashCacheService()
        final Server server
        final ManagedChannel channel

        TestFileHashCacheHarness() {
            tempDir = Files.createTempDirectory("rust-file-hash-cache-test").toFile()
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

        FileHashCacheServiceGrpc.FileHashCacheServiceBlockingStub blockingStub() {
            FileHashCacheServiceGrpc.newBlockingStub(channel)
        }

        @Override
        void close() {
            channel.shutdownNow()
            server.shutdownNow()
            server.awaitTermination()
            channel.awaitTermination(5, java.util.concurrent.TimeUnit.SECONDS)
            tempDir.deleteDir()
        }
    }

    def "getFileInfo returns noop miss without touching stubs"() {
        given:
        def substrate = SubstrateClient.noop("disabled")
        def client = new RustFileHashCacheClient(substrate)

        expect:
        !client.getFileInfo("/tmp/input.txt", 12L, 34L, "FILE_HASHES").hit
    }

    def "getFileInfo maps rust hit to file info data"() {
        given:
        def harness = new TestFileHashCacheHarness()
        def substrate = Mock(SubstrateClient)
        def rustHash = "0123456789abcdef".bytes
        substrate.isNoop() >> false
        substrate.getFileHashCacheStub() >> harness.blockingStub()
        harness.service.getResponse = GetFileInfoResponse.newBuilder()
            .setHit(true)
            .setInfo(FileInfo.newBuilder()
                .setHash(ByteString.copyFrom(rustHash))
                .setLength(12L)
                .setLastModified(34L)
                .build())
            .build()
        def client = new RustFileHashCacheClient(substrate)

        when:
        def result = client.getFileInfo("/tmp/input.txt", 12L, 34L, null)

        then:
        result.hit
        result.info.get().hash == HashCode.fromBytes(rustHash)
        result.info.get().length == 12L
        result.info.get().lastModified == 34L
        harness.service.lastGet.path == "/tmp/input.txt"
        harness.service.lastGet.kind == "FILE_HASHES"

        cleanup:
        harness.close()
    }

    def "putFileInfo sends hash bytes to rust"() {
        given:
        def harness = new TestFileHashCacheHarness()
        def substrate = Mock(SubstrateClient)
        def hash = HashCode.fromBytes("0123456789abcdef".bytes)
        substrate.isNoop() >> false
        substrate.getFileHashCacheStub() >> harness.blockingStub()
        def client = new RustFileHashCacheClient(substrate)

        expect:
        client.putFileInfo("/tmp/input.txt", hash, 12L, 34L, "CHECKSUMS")
        harness.service.lastPut.path == "/tmp/input.txt"
        harness.service.lastPut.kind == "CHECKSUMS"
        harness.service.lastPut.info.hash.toByteArray() == hash.toByteArray()
        harness.service.lastPut.info.length == 12L
        harness.service.lastPut.info.lastModified == 34L

        cleanup:
        harness.close()
    }

    def "invalidate and stats delegate to rust"() {
        given:
        def harness = new TestFileHashCacheHarness()
        def substrate = Mock(SubstrateClient)
        substrate.isNoop() >> false
        substrate.getFileHashCacheStub() >> harness.blockingStub()
        harness.service.statsResponse = GetFileHashCacheStatsResponse.newBuilder()
            .setHits(3L)
            .setMisses(4L)
            .setEntries(5L)
            .setBytes(6L)
            .build()
        def client = new RustFileHashCacheClient(substrate)

        when:
        def invalidateResult = client.invalidate("/tmp/input.txt")
        def stats = client.stats

        then:
        invalidateResult
        harness.service.lastInvalidate.path == "/tmp/input.txt"
        stats.hits == 3L
        stats.misses == 4L
        stats.entries == 5L
        stats.bytes == 6L

        cleanup:
        harness.close()
    }
}
