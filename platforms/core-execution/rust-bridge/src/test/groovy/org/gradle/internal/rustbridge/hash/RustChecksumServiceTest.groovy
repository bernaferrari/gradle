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
import org.gradle.internal.hash.ChecksumService
import org.gradle.internal.hash.HashCode
import org.gradle.internal.rustbridge.SubstrateClient
import org.gradle.internal.rustbridge.SubstrateException
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

import java.nio.file.Files

class RustChecksumServiceTest extends Specification {

    private static byte[] hashBytes(int seed, int length) {
        (0..<length).collect { (byte) (seed + it) } as byte[]
    }

    private static HashCode hash(int seed, int length = 16) {
        HashCode.fromBytes(hashBytes(seed, length))
    }

    private static class TestHashService extends HashServiceGrpc.HashServiceImplBase {
        HashBatchRequest lastRequest
        byte[] responseHash = hashBytes(11, 16)

        @Override
        void hashBatch(HashBatchRequest request, StreamObserver<HashBatchResponse> responseObserver) {
            lastRequest = request
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

    def "md5 checksum uses raw Rust md5 without Gradle stream signature"() {
        given:
        def harness = new TestHashHarness()
        def file = Files.createTempFile("rust-checksum-md5", ".bin").toFile()
        file.text = "checksum"
        def substrate = Mock(SubstrateClient)
        substrate.getHashStub() >> harness.blockingStub()
        def service = new RustChecksumService(substrate)

        when:
        def result = service.md5(file)

        then:
        result == HashCode.fromBytes(harness.service.responseHash)
        harness.service.lastRequest.algorithm == "MD5"
        !harness.service.lastRequest.gradleSignature

        cleanup:
        harness.close()
        file.delete()
    }

    def "sha512 checksum delegates to Rust SHA-512"() {
        given:
        def harness = new TestHashHarness()
        harness.service.responseHash = hashBytes(19, 64)
        def file = Files.createTempFile("rust-checksum-sha512", ".bin").toFile()
        file.text = "checksum"
        def substrate = Mock(SubstrateClient)
        substrate.getHashStub() >> harness.blockingStub()
        def service = new RustChecksumService(substrate)

        when:
        def result = service.sha512(file)

        then:
        result == HashCode.fromBytes(harness.service.responseHash)
        harness.service.lastRequest.algorithm == "SHA-512"
        !harness.service.lastRequest.gradleSignature

        cleanup:
        harness.close()
        file.delete()
    }

    def "shadow checksum returns Java hash and reports mismatch"() {
        given:
        def file = Files.createTempFile("shadow-checksum", ".bin").toFile()
        def java = Mock(ChecksumService)
        def rust = Mock(ChecksumService)
        def reporter = new HashMismatchReporter(false)
        def service = new ShadowingChecksumService(java, rust, reporter, false)
        java.hash(file, "sha256") >> hash(1, 32)
        rust.hash(file, "sha256") >> hash(2, 32)

        expect:
        service.hash(file, "sha256") == hash(1, 32)
        reporter.totalMismatches == 1

        cleanup:
        file.delete()
    }

    def "authoritative checksum fails closed on mismatch"() {
        given:
        def file = Files.createTempFile("authoritative-checksum", ".bin").toFile()
        def java = Mock(ChecksumService)
        def rust = Mock(ChecksumService)
        def service = new ShadowingChecksumService(java, rust, new HashMismatchReporter(false), true)
        rust.hash(file, "sha1") >> hash(3, 20)
        java.hash(file, "sha1") >> hash(4, 20)

        when:
        service.hash(file, "sha1")

        then:
        thrown(SubstrateException)

        cleanup:
        file.delete()
    }
}
