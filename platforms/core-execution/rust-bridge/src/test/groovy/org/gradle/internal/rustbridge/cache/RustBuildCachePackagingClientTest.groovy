package org.gradle.internal.rustbridge.cache

import com.google.protobuf.ByteString
import gradle.substrate.v1.BuildCachePackFile
import gradle.substrate.v1.BuildCachePackagingServiceGrpc
import gradle.substrate.v1.PackCacheEntryRequest
import gradle.substrate.v1.PackCacheEntryResponse
import gradle.substrate.v1.UnpackCacheEntryRequest
import gradle.substrate.v1.UnpackCacheEntryResponse
import io.grpc.ManagedChannel
import io.grpc.ManagedChannelBuilder
import io.grpc.Server
import io.grpc.netty.shaded.io.grpc.netty.NettyServerBuilder
import io.grpc.stub.StreamObserver
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class RustBuildCachePackagingClientTest extends Specification {

    private static class TestPackagingService extends BuildCachePackagingServiceGrpc.BuildCachePackagingServiceImplBase {
        PackCacheEntryRequest lastPackRequest
        UnpackCacheEntryRequest lastUnpackRequest
        boolean failPack
        boolean failUnpack

        @Override
        void packCacheEntry(PackCacheEntryRequest request, StreamObserver<PackCacheEntryResponse> responseObserver) {
            lastPackRequest = request
            if (failPack) {
                responseObserver.onNext(PackCacheEntryResponse.newBuilder()
                    .setSuccess(false)
                    .setError("pack failed")
                    .build())
            } else {
                responseObserver.onNext(PackCacheEntryResponse.newBuilder()
                    .setSuccess(true)
                    .setPackagedBytes(ByteString.copyFrom("packed:${request.buildId}".getBytes("UTF-8")))
                    .setEntryCount(request.filesCount + 1)
                    .build())
            }
            responseObserver.onCompleted()
        }

        @Override
        void unpackCacheEntry(UnpackCacheEntryRequest request, StreamObserver<UnpackCacheEntryResponse> responseObserver) {
            lastUnpackRequest = request
            if (failUnpack) {
                responseObserver.onNext(UnpackCacheEntryResponse.newBuilder()
                    .setSuccess(false)
                    .setError("unpack failed")
                    .build())
            } else {
                responseObserver.onNext(UnpackCacheEntryResponse.newBuilder()
                    .setSuccess(true)
                    .addFiles(BuildCachePackFile.newBuilder()
                        .setPath("classes/App.class")
                        .setContent(ByteString.copyFrom([0xca, 0xfe] as byte[]))
                        .setExecutable(false)
                        .build())
                    .addFiles(BuildCachePackFile.newBuilder()
                        .setPath("bin/run")
                        .setContent(ByteString.copyFrom("run".getBytes("UTF-8")))
                        .setExecutable(true)
                        .build())
                    .putOriginMetadata("identity", ":compileJava")
                    .setEntryCount(3)
                    .build())
            }
            responseObserver.onCompleted()
        }
    }

    private static class TestPackagingHarness implements Closeable {
        final TestPackagingService service = new TestPackagingService()
        final Server server
        final ManagedChannel channel

        TestPackagingHarness() {
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

        BuildCachePackagingServiceGrpc.BuildCachePackagingServiceBlockingStub blockingStub() {
            BuildCachePackagingServiceGrpc.newBlockingStub(channel)
        }

        @Override
        void close() {
            channel.shutdownNow()
            server.shutdownNow()
            server.awaitTermination()
            channel.awaitTermination(5, java.util.concurrent.TimeUnit.SECONDS)
        }
    }

    def "noop client fails closed"() {
        given:
        def client = new RustBuildCachePackagingClient(SubstrateClient.noop())

        expect:
        !client.packCacheEntry("build", "content").success
        client.packCacheEntry("build", "content").error == "substrate-disabled"
    }

    def "pack delegates typed spec to Rust service"() {
        given:
        def harness = new TestPackagingHarness()
        def substrate = Mock(SubstrateClient)
        substrate.isNoop() >> false
        substrate.getCachePackagingStub() >> harness.blockingStub()
        def client = new RustBuildCachePackagingClient(substrate)
        def spec = RustBuildCachePackagingClient.PackSpec.of(
            [
                new RustBuildCachePackagingClient.Entry("classes/App.class", [0xca, 0xfe] as byte[], false),
                new RustBuildCachePackagingClient.Entry("bin/run", "run".getBytes("UTF-8"), true)
            ],
            [identity: ":compileJava"]
        )

        when:
        def result = client.packCacheEntry("build-1", spec)

        then:
        result.success
        new String(result.packagedBytes, "UTF-8") == "packed:build-1"
        result.entryCount == 3
        harness.service.lastPackRequest.buildId == "build-1"
        harness.service.lastPackRequest.gzip
        harness.service.lastPackRequest.originMetadataMap.identity == ":compileJava"
        harness.service.lastPackRequest.filesCount == 2
        harness.service.lastPackRequest.getFiles(1).path == "bin/run"
        harness.service.lastPackRequest.getFiles(1).executable

        cleanup:
        harness.close()
    }

    def "unpack maps Rust response entries and metadata"() {
        given:
        def harness = new TestPackagingHarness()
        def substrate = Mock(SubstrateClient)
        substrate.isNoop() >> false
        substrate.getCachePackagingStub() >> harness.blockingStub()
        def client = new RustBuildCachePackagingClient(substrate)

        when:
        def result = client.unpackCacheEntry("build-1", "packed".getBytes("UTF-8"), true)

        then:
        result.success
        result.entryCount == 3
        result.originMetadata.identity == ":compileJava"
        result.entries*.path == ["classes/App.class", "bin/run"]
        result.entries[1].executable
        harness.service.lastUnpackRequest.gzip

        cleanup:
        harness.close()
    }

    def "pack failure is observable and fail closed"() {
        given:
        def harness = new TestPackagingHarness()
        harness.service.failPack = true
        def substrate = Mock(SubstrateClient)
        substrate.isNoop() >> false
        substrate.getCachePackagingStub() >> harness.blockingStub()
        def client = new RustBuildCachePackagingClient(substrate)

        when:
        def result = client.packCacheEntry("build-1", "content")

        then:
        !result.success
        result.error == "pack failed"

        cleanup:
        harness.close()
    }
}
