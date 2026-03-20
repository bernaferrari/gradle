package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class CacheServiceGrpc {

  private CacheServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.CacheService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.CacheLoadRequest,
      gradle.substrate.v1.Substrate.CacheLoadChunk> getLoadEntryMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "LoadEntry",
      requestType = gradle.substrate.v1.Substrate.CacheLoadRequest.class,
      responseType = gradle.substrate.v1.Substrate.CacheLoadChunk.class,
      methodType = io.grpc.MethodDescriptor.MethodType.SERVER_STREAMING)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.CacheLoadRequest,
      gradle.substrate.v1.Substrate.CacheLoadChunk> getLoadEntryMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.CacheLoadRequest, gradle.substrate.v1.Substrate.CacheLoadChunk> getLoadEntryMethod;
    if ((getLoadEntryMethod = CacheServiceGrpc.getLoadEntryMethod) == null) {
      synchronized (CacheServiceGrpc.class) {
        if ((getLoadEntryMethod = CacheServiceGrpc.getLoadEntryMethod) == null) {
          CacheServiceGrpc.getLoadEntryMethod = getLoadEntryMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.CacheLoadRequest, gradle.substrate.v1.Substrate.CacheLoadChunk>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.SERVER_STREAMING)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "LoadEntry"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.CacheLoadRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.CacheLoadChunk.getDefaultInstance()))
              .setSchemaDescriptor(new CacheServiceMethodDescriptorSupplier("LoadEntry"))
              .build();
        }
      }
    }
    return getLoadEntryMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.CacheStoreChunk,
      gradle.substrate.v1.Substrate.CacheStoreResponse> getStoreEntryMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "StoreEntry",
      requestType = gradle.substrate.v1.Substrate.CacheStoreChunk.class,
      responseType = gradle.substrate.v1.Substrate.CacheStoreResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.CLIENT_STREAMING)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.CacheStoreChunk,
      gradle.substrate.v1.Substrate.CacheStoreResponse> getStoreEntryMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.CacheStoreChunk, gradle.substrate.v1.Substrate.CacheStoreResponse> getStoreEntryMethod;
    if ((getStoreEntryMethod = CacheServiceGrpc.getStoreEntryMethod) == null) {
      synchronized (CacheServiceGrpc.class) {
        if ((getStoreEntryMethod = CacheServiceGrpc.getStoreEntryMethod) == null) {
          CacheServiceGrpc.getStoreEntryMethod = getStoreEntryMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.CacheStoreChunk, gradle.substrate.v1.Substrate.CacheStoreResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.CLIENT_STREAMING)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "StoreEntry"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.CacheStoreChunk.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.CacheStoreResponse.getDefaultInstance()))
              .setSchemaDescriptor(new CacheServiceMethodDescriptorSupplier("StoreEntry"))
              .build();
        }
      }
    }
    return getStoreEntryMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static CacheServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<CacheServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<CacheServiceStub>() {
        @java.lang.Override
        public CacheServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new CacheServiceStub(channel, callOptions);
        }
      };
    return CacheServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static CacheServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<CacheServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<CacheServiceBlockingV2Stub>() {
        @java.lang.Override
        public CacheServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new CacheServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return CacheServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static CacheServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<CacheServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<CacheServiceBlockingStub>() {
        @java.lang.Override
        public CacheServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new CacheServiceBlockingStub(channel, callOptions);
        }
      };
    return CacheServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static CacheServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<CacheServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<CacheServiceFutureStub>() {
        @java.lang.Override
        public CacheServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new CacheServiceFutureStub(channel, callOptions);
        }
      };
    return CacheServiceFutureStub.newStub(factory, channel);
  }

  /**
   */
  public interface AsyncService {

    /**
     */
    default void loadEntry(gradle.substrate.v1.Substrate.CacheLoadRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheLoadChunk> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getLoadEntryMethod(), responseObserver);
    }

    /**
     */
    default io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheStoreChunk> storeEntry(
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheStoreResponse> responseObserver) {
      return io.grpc.stub.ServerCalls.asyncUnimplementedStreamingCall(getStoreEntryMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service CacheService.
   */
  public static abstract class CacheServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return CacheServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service CacheService.
   */
  public static final class CacheServiceStub
      extends io.grpc.stub.AbstractAsyncStub<CacheServiceStub> {
    private CacheServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected CacheServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new CacheServiceStub(channel, callOptions);
    }

    /**
     */
    public void loadEntry(gradle.substrate.v1.Substrate.CacheLoadRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheLoadChunk> responseObserver) {
      io.grpc.stub.ClientCalls.asyncServerStreamingCall(
          getChannel().newCall(getLoadEntryMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     */
    public io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheStoreChunk> storeEntry(
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheStoreResponse> responseObserver) {
      return io.grpc.stub.ClientCalls.asyncClientStreamingCall(
          getChannel().newCall(getStoreEntryMethod(), getCallOptions()), responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service CacheService.
   */
  public static final class CacheServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<CacheServiceBlockingV2Stub> {
    private CacheServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected CacheServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new CacheServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     */
    @io.grpc.ExperimentalApi("https://github.com/grpc/grpc-java/issues/10918")
    public io.grpc.stub.BlockingClientCall<?, gradle.substrate.v1.Substrate.CacheLoadChunk>
        loadEntry(gradle.substrate.v1.Substrate.CacheLoadRequest request) {
      return io.grpc.stub.ClientCalls.blockingV2ServerStreamingCall(
          getChannel(), getLoadEntryMethod(), getCallOptions(), request);
    }

    /**
     */
    @io.grpc.ExperimentalApi("https://github.com/grpc/grpc-java/issues/10918")
    public io.grpc.stub.BlockingClientCall<gradle.substrate.v1.Substrate.CacheStoreChunk, gradle.substrate.v1.Substrate.CacheStoreResponse>
        storeEntry() {
      return io.grpc.stub.ClientCalls.blockingClientStreamingCall(
          getChannel(), getStoreEntryMethod(), getCallOptions());
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service CacheService.
   */
  public static final class CacheServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<CacheServiceBlockingStub> {
    private CacheServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected CacheServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new CacheServiceBlockingStub(channel, callOptions);
    }

    /**
     */
    public java.util.Iterator<gradle.substrate.v1.Substrate.CacheLoadChunk> loadEntry(
        gradle.substrate.v1.Substrate.CacheLoadRequest request) {
      return io.grpc.stub.ClientCalls.blockingServerStreamingCall(
          getChannel(), getLoadEntryMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service CacheService.
   */
  public static final class CacheServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<CacheServiceFutureStub> {
    private CacheServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected CacheServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new CacheServiceFutureStub(channel, callOptions);
    }
  }

  private static final int METHODID_LOAD_ENTRY = 0;
  private static final int METHODID_STORE_ENTRY = 1;

  private static final class MethodHandlers<Req, Resp> implements
      io.grpc.stub.ServerCalls.UnaryMethod<Req, Resp>,
      io.grpc.stub.ServerCalls.ServerStreamingMethod<Req, Resp>,
      io.grpc.stub.ServerCalls.ClientStreamingMethod<Req, Resp>,
      io.grpc.stub.ServerCalls.BidiStreamingMethod<Req, Resp> {
    private final AsyncService serviceImpl;
    private final int methodId;

    MethodHandlers(AsyncService serviceImpl, int methodId) {
      this.serviceImpl = serviceImpl;
      this.methodId = methodId;
    }

    @java.lang.Override
    @java.lang.SuppressWarnings("unchecked")
    public void invoke(Req request, io.grpc.stub.StreamObserver<Resp> responseObserver) {
      switch (methodId) {
        case METHODID_LOAD_ENTRY:
          serviceImpl.loadEntry((gradle.substrate.v1.Substrate.CacheLoadRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheLoadChunk>) responseObserver);
          break;
        default:
          throw new AssertionError();
      }
    }

    @java.lang.Override
    @java.lang.SuppressWarnings("unchecked")
    public io.grpc.stub.StreamObserver<Req> invoke(
        io.grpc.stub.StreamObserver<Resp> responseObserver) {
      switch (methodId) {
        case METHODID_STORE_ENTRY:
          return (io.grpc.stub.StreamObserver<Req>) serviceImpl.storeEntry(
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.CacheStoreResponse>) responseObserver);
        default:
          throw new AssertionError();
      }
    }
  }

  public static final io.grpc.ServerServiceDefinition bindService(AsyncService service) {
    return io.grpc.ServerServiceDefinition.builder(getServiceDescriptor())
        .addMethod(
          getLoadEntryMethod(),
          io.grpc.stub.ServerCalls.asyncServerStreamingCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.CacheLoadRequest,
              gradle.substrate.v1.Substrate.CacheLoadChunk>(
                service, METHODID_LOAD_ENTRY)))
        .addMethod(
          getStoreEntryMethod(),
          io.grpc.stub.ServerCalls.asyncClientStreamingCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.CacheStoreChunk,
              gradle.substrate.v1.Substrate.CacheStoreResponse>(
                service, METHODID_STORE_ENTRY)))
        .build();
  }

  private static abstract class CacheServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    CacheServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("CacheService");
    }
  }

  private static final class CacheServiceFileDescriptorSupplier
      extends CacheServiceBaseDescriptorSupplier {
    CacheServiceFileDescriptorSupplier() {}
  }

  private static final class CacheServiceMethodDescriptorSupplier
      extends CacheServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    CacheServiceMethodDescriptorSupplier(java.lang.String methodName) {
      this.methodName = methodName;
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.MethodDescriptor getMethodDescriptor() {
      return getServiceDescriptor().findMethodByName(methodName);
    }
  }

  private static volatile io.grpc.ServiceDescriptor serviceDescriptor;

  public static io.grpc.ServiceDescriptor getServiceDescriptor() {
    io.grpc.ServiceDescriptor result = serviceDescriptor;
    if (result == null) {
      synchronized (CacheServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new CacheServiceFileDescriptorSupplier())
              .addMethod(getLoadEntryMethod())
              .addMethod(getStoreEntryMethod())
              .build();
        }
      }
    }
    return result;
  }
}
