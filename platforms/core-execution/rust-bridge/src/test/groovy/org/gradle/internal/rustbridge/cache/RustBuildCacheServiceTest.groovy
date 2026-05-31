package org.gradle.internal.rustbridge.cache

import com.google.protobuf.ByteString
import gradle.substrate.v1.CacheLoadChunk
import gradle.substrate.v1.CacheLoadRequest
import gradle.substrate.v1.CacheServiceGrpc
import gradle.substrate.v1.CacheStoreChunk
import gradle.substrate.v1.CacheStoreInit
import gradle.substrate.v1.CacheStoreResponse
import io.grpc.ManagedChannel
import io.grpc.ManagedChannelBuilder
import io.grpc.Server
import io.grpc.netty.shaded.io.grpc.netty.NettyServerBuilder
import io.grpc.stub.StreamObserver
import org.gradle.caching.BuildCacheEntryReader
import org.gradle.caching.BuildCacheEntryWriter
import org.gradle.caching.BuildCacheException
import org.gradle.caching.BuildCacheKey
import org.gradle.caching.BuildCacheService
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

import java.nio.file.Files
import java.util.concurrent.ConcurrentHashMap

class RustBuildCacheServiceTest extends Specification {

    private static class TestCacheService extends CacheServiceGrpc.CacheServiceImplBase {
        final Map<String, byte[]> entries = new ConcurrentHashMap<>()
        final Map<String, Integer> dataChunkCounts = new ConcurrentHashMap<>()
        volatile boolean failStores = false

        void put(byte[] key, byte[] data) {
            entries.put(keyId(key), data)
        }

        byte[] get(byte[] key) {
            entries.get(keyId(key))
        }

        @Override
        void loadEntry(CacheLoadRequest request, StreamObserver<CacheLoadChunk> responseObserver) {
            def data = entries.get(keyId(request.key.toByteArray()))
            if (data != null) {
                responseObserver.onNext(CacheLoadChunk.newBuilder().setData(ByteString.copyFrom(data)).build())
            }
            responseObserver.onCompleted()
        }

        @Override
        StreamObserver<CacheStoreChunk> storeEntry(StreamObserver<CacheStoreResponse> responseObserver) {
            return new StreamObserver<CacheStoreChunk>() {
                private byte[] key = new byte[0]
                private final ByteArrayOutputStream data = new ByteArrayOutputStream()

                @Override
                void onNext(CacheStoreChunk chunk) {
                    if (chunk.hasInit()) {
                        CacheStoreInit init = chunk.getInit()
                        key = init.key.toByteArray()
                    } else if (chunk.hasData()) {
                        def id = keyId(key)
                        dataChunkCounts.put(id, (dataChunkCounts.get(id) ?: 0) + 1)
                        data.write(chunk.getData().toByteArray())
                    }
                }

                @Override
                void onError(Throwable t) {
                    responseObserver.onCompleted()
                }

                @Override
                void onCompleted() {
                    if (failStores) {
                        responseObserver.onNext(CacheStoreResponse.newBuilder()
                            .setSuccess(false)
                            .setErrorMessage("disk full")
                            .build())
                    } else {
                        entries.put(keyId(key), data.toByteArray())
                        responseObserver.onNext(CacheStoreResponse.newBuilder()
                            .setSuccess(true)
                            .setErrorMessage("")
                            .build())
                    }
                    responseObserver.onCompleted()
                }
            }
        }

        private static String keyId(byte[] key) {
            ByteString.copyFrom(key).toStringUtf8()
        }
    }

    private static class TestCacheHarness implements Closeable {
        final File tempDir
        final TestCacheService service = new TestCacheService()
        final Server server
        final ManagedChannel channel

        TestCacheHarness() {
            tempDir = Files.createTempDirectory("rust-cache-test").toFile()
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

        CacheServiceGrpc.CacheServiceBlockingStub blockingStub() {
            CacheServiceGrpc.newBlockingStub(channel)
        }

        CacheServiceGrpc.CacheServiceStub asyncStub() {
            CacheServiceGrpc.newStub(channel)
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

    def "implements BuildCacheService interface"() {
        expect:
        BuildCacheService.isAssignableFrom(RustBuildCacheService)
    }

    def "load returns false when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def service = new RustBuildCacheService(client)
        def key = Mock(BuildCacheKey)
        def reader = Mock(BuildCacheEntryReader)

        expect:
        service.load(key, reader) == false
    }

    def "store does nothing when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def service = new RustBuildCacheService(client)
        def key = Mock(BuildCacheKey)
        def writer = Mock(BuildCacheEntryWriter)

        when:
        service.store(key, writer)

        then:
        noExceptionThrown()
        0 * writer._
    }

    def "load returns false for cache miss"() {
        given:
        def harness = new TestCacheHarness()
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheStub() >> harness.blockingStub()
        def service = new RustBuildCacheService(client)
        def key = Mock(BuildCacheKey)
        key.toByteArray() >> "missing-key".getBytes("UTF-8")
        def reader = Mock(BuildCacheEntryReader)

        when:
        def result = service.load(key, reader)

        then:
        result == false
        0 * reader._

        cleanup:
        harness.close()
    }

    def "load returns true and reads data for cache hit"() {
        given:
        def harness = new TestCacheHarness()
        def chunkData = "hello world".getBytes("UTF-8")
        def keyBytes = "cache-hit-key".getBytes("UTF-8")
        harness.service.put(keyBytes, chunkData)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheStub() >> harness.blockingStub()
        def service = new RustBuildCacheService(client)
        def key = Mock(BuildCacheKey)
        key.toByteArray() >> keyBytes
        def reader = Mock(BuildCacheEntryReader)

        when:
        def result = service.load(key, reader)

        then:
        result == true
        1 * reader.readFrom(_ as InputStream) >> { InputStream is ->
            assert is.bytes == chunkData
        }

        cleanup:
        harness.close()
    }

    def "store sends init and data chunks to the cache server"() {
        given:
        def harness = new TestCacheHarness()
        def storeData = "cached content".getBytes("UTF-8")
        def keyBytes = "cache-key-bytes".getBytes("UTF-8")
        def key = Mock(BuildCacheKey)
        key.toByteArray() >> keyBytes
        def writer = Mock(BuildCacheEntryWriter)
        writer.writeTo(_ as OutputStream) >> { OutputStream os ->
            os.write(storeData)
        }

        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheAsyncStub() >> harness.asyncStub()
        def service = new RustBuildCacheService(client)

        when:
        service.store(key, writer)

        then:
        noExceptionThrown()
        harness.service.get(keyBytes) == storeData

        cleanup:
        harness.close()
    }

    def "store splits large entries into multiple data chunks"() {
        given:
        def harness = new TestCacheHarness()
        def storeData = new byte[150_000]
        Arrays.fill(storeData, (byte) 7)
        def keyBytes = "large-cache-key".getBytes("UTF-8")
        def key = Mock(BuildCacheKey)
        key.toByteArray() >> keyBytes
        def writer = Mock(BuildCacheEntryWriter)
        writer.writeTo(_ as OutputStream) >> { OutputStream os ->
            os.write(storeData)
        }

        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheAsyncStub() >> harness.asyncStub()
        def service = new RustBuildCacheService(client)

        when:
        service.store(key, writer)

        then:
        noExceptionThrown()
        harness.service.get(keyBytes) == storeData
        harness.service.dataChunkCounts.get(new String(keyBytes, "UTF-8")) > 1

        cleanup:
        harness.close()
    }

    def "store throws BuildCacheException on failure"() {
        given:
        def harness = new TestCacheHarness()
        harness.service.failStores = true
        def key = Mock(BuildCacheKey)
        key.toByteArray() >> "bad-key".getBytes("UTF-8")
        def writer = Mock(BuildCacheEntryWriter)
        writer.writeTo(_ as OutputStream) >> {}

        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheAsyncStub() >> harness.asyncStub()
        def service = new RustBuildCacheService(client)

        when:
        service.store(key, writer)

        then:
        def ex = thrown(BuildCacheException)
        ex.message.contains("disk full")

        cleanup:
        harness.close()
    }

    def "close does nothing"() {
        given:
        def client = Mock(SubstrateClient)
        def service = new RustBuildCacheService(client)

        when:
        service.close()

        then:
        noExceptionThrown()
        0 * client._
    }
}
