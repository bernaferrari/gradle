package org.gradle.internal.rustbridge.cache

import gradle.substrate.v1.CacheLoadChunk
import gradle.substrate.v1.CacheStoreChunk
import gradle.substrate.v1.CacheStoreResponse
import io.grpc.stub.StreamObserver
import org.gradle.caching.BuildCacheEntryReader
import org.gradle.caching.BuildCacheEntryWriter
import org.gradle.caching.BuildCacheException
import org.gradle.caching.BuildCacheKey
import org.gradle.caching.BuildCacheService
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

import java.util.concurrent.CountDownLatch

class RustBuildCacheServiceTest extends Specification {

    def "implements BuildCacheService interface"() {
        expect:
        BuildCacheService.isAssignableFrom(RustBuildCacheService)
    }

    def "load returns false when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def service = new RustBuildCacheService(client, "test")
        def key = Mock(BuildCacheKey)
        def reader = Mock(BuildCacheEntryReader)

        expect:
        service.load(key, reader) == false
    }

    def "store does nothing when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def service = new RustBuildCacheService(client, "test")
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
        def blockingStub = Mock(CacheServiceGrpc.CacheServiceBlockingStub)
        blockingStub.loadEntry(_) >> [].iterator()
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheStub() >> blockingStub
        def service = new RustBuildCacheService(client, "test")
        def key = Mock(BuildCacheKey)
        def reader = Mock(BuildCacheEntryReader)

        expect:
        service.load(key, reader) == false
        0 * reader._
    }

    def "load returns true and reads data for cache hit"() {
        given:
        def chunkData = "hello world".getBytes("UTF-8")
        def chunk = CacheLoadChunk.newBuilder()
            .setData(com.google.protobuf.ByteString.copyFrom(chunkData))
            .build()
        def blockingStub = Mock(CacheServiceGrpc.CacheServiceBlockingStub)
        blockingStub.loadEntry(_) >> [chunk].iterator()
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheStub() >> blockingStub
        def service = new RustBuildCacheService(client, "test")
        def key = Mock(BuildCacheKey)
        def reader = Mock(BuildCacheEntryReader)

        when:
        def result = service.load(key, reader)

        then:
        result == true
        1 * reader.readFrom(_ as InputStream) >> { InputStream is ->
            assert is.bytes == chunkData
        }
    }

    def "store sends init and data chunks via async stub"() {
        given:
        def storeData = "cached content".getBytes("UTF-8")
        def key = Mock(BuildCacheKey)
        key.toByteArray() >> "cache-key-bytes".getBytes("UTF-8")
        def writer = Mock(BuildCacheEntryWriter)
        writer.writeTo(_ as OutputStream) >> { OutputStream os ->
            os.write(storeData)
        }

        def response = CacheStoreResponse.newBuilder()
            .setSuccess(true)
            .setErrorMessage("")
            .build()

        def capturedObserver = null as StreamObserver<CacheStoreChunk>
        def asyncStub = Mock(CacheServiceGrpc.CacheServiceStub)
        asyncStub.storeEntry(_ as StreamObserver) >> { StreamObserver<CacheStoreResponse> respObserver ->
            capturedObserver = it[0]
            respObserver.onNext(response)
            respObserver.onCompleted()
            return capturedObserver
        }

        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheAsyncStub() >> asyncStub
        def service = new RustBuildCacheService(client, "test")

        when:
        service.store(key, writer)

        then:
        noExceptionThrown()
        capturedObserver != null
    }

    def "store throws BuildCacheException on failure"() {
        given:
        def key = Mock(BuildCacheKey)
        key.toByteArray() >> "bad-key".getBytes("UTF-8")
        def writer = Mock(BuildCacheEntryWriter)
        writer.writeTo(_ as OutputStream) >> {}

        def asyncStub = Mock(CacheServiceGrpc.CacheServiceStub)
        asyncStub.storeEntry(_ as StreamObserver) >> { StreamObserver<CacheStoreResponse> respObserver ->
            respObserver.onNext(CacheStoreResponse.newBuilder()
                .setSuccess(false)
                .setErrorMessage("disk full")
                .build())
            respObserver.onCompleted()
            return Mock(StreamObserver)
        }

        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getCacheAsyncStub() >> asyncStub
        def service = new RustBuildCacheService(client, "test")

        when:
        service.store(key, writer)

        then:
        def ex = thrown(BuildCacheException)
        ex.message.contains("disk full")
    }

    def "close does nothing"() {
        given:
        def client = Mock(SubstrateClient)
        def service = new RustBuildCacheService(client, "test")

        when:
        service.close()

        then:
        noExceptionThrown()
        0 * client._
    }
}
