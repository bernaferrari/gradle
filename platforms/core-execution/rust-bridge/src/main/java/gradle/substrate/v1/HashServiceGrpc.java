package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class HashServiceGrpc {

  private HashServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.HashService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.HashBatchRequest,
      gradle.substrate.v1.Substrate.HashBatchResponse> getHashBatchMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "HashBatch",
      requestType = gradle.substrate.v1.Substrate.HashBatchRequest.class,
      responseType = gradle.substrate.v1.Substrate.HashBatchResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.HashBatchRequest,
      gradle.substrate.v1.Substrate.HashBatchResponse> getHashBatchMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.HashBatchRequest, gradle.substrate.v1.Substrate.HashBatchResponse> getHashBatchMethod;
    if ((getHashBatchMethod = HashServiceGrpc.getHashBatchMethod) == null) {
      synchronized (HashServiceGrpc.class) {
        if ((getHashBatchMethod = HashServiceGrpc.getHashBatchMethod) == null) {
          HashServiceGrpc.getHashBatchMethod = getHashBatchMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.HashBatchRequest, gradle.substrate.v1.Substrate.HashBatchResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "HashBatch"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.HashBatchRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.HashBatchResponse.getDefaultInstance()))
              .setSchemaDescriptor(new HashServiceMethodDescriptorSupplier("HashBatch"))
              .build();
        }
      }
    }
    return getHashBatchMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static HashServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<HashServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<HashServiceStub>() {
        @java.lang.Override
        public HashServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new HashServiceStub(channel, callOptions);
        }
      };
    return HashServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static HashServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<HashServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<HashServiceBlockingV2Stub>() {
        @java.lang.Override
        public HashServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new HashServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return HashServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static HashServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<HashServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<HashServiceBlockingStub>() {
        @java.lang.Override
        public HashServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new HashServiceBlockingStub(channel, callOptions);
        }
      };
    return HashServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static HashServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<HashServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<HashServiceFutureStub>() {
        @java.lang.Override
        public HashServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new HashServiceFutureStub(channel, callOptions);
        }
      };
    return HashServiceFutureStub.newStub(factory, channel);
  }

  /**
   */
  public interface AsyncService {

    /**
     */
    default void hashBatch(gradle.substrate.v1.Substrate.HashBatchRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.HashBatchResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getHashBatchMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service HashService.
   */
  public static abstract class HashServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return HashServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service HashService.
   */
  public static final class HashServiceStub
      extends io.grpc.stub.AbstractAsyncStub<HashServiceStub> {
    private HashServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected HashServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new HashServiceStub(channel, callOptions);
    }

    /**
     */
    public void hashBatch(gradle.substrate.v1.Substrate.HashBatchRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.HashBatchResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getHashBatchMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service HashService.
   */
  public static final class HashServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<HashServiceBlockingV2Stub> {
    private HashServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected HashServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new HashServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.HashBatchResponse hashBatch(gradle.substrate.v1.Substrate.HashBatchRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getHashBatchMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service HashService.
   */
  public static final class HashServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<HashServiceBlockingStub> {
    private HashServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected HashServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new HashServiceBlockingStub(channel, callOptions);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.HashBatchResponse hashBatch(gradle.substrate.v1.Substrate.HashBatchRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getHashBatchMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service HashService.
   */
  public static final class HashServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<HashServiceFutureStub> {
    private HashServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected HashServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new HashServiceFutureStub(channel, callOptions);
    }

    /**
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.HashBatchResponse> hashBatch(
        gradle.substrate.v1.Substrate.HashBatchRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getHashBatchMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_HASH_BATCH = 0;

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
        case METHODID_HASH_BATCH:
          serviceImpl.hashBatch((gradle.substrate.v1.Substrate.HashBatchRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.HashBatchResponse>) responseObserver);
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
        default:
          throw new AssertionError();
      }
    }
  }

  public static final io.grpc.ServerServiceDefinition bindService(AsyncService service) {
    return io.grpc.ServerServiceDefinition.builder(getServiceDescriptor())
        .addMethod(
          getHashBatchMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.HashBatchRequest,
              gradle.substrate.v1.Substrate.HashBatchResponse>(
                service, METHODID_HASH_BATCH)))
        .build();
  }

  private static abstract class HashServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    HashServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("HashService");
    }
  }

  private static final class HashServiceFileDescriptorSupplier
      extends HashServiceBaseDescriptorSupplier {
    HashServiceFileDescriptorSupplier() {}
  }

  private static final class HashServiceMethodDescriptorSupplier
      extends HashServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    HashServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (HashServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new HashServiceFileDescriptorSupplier())
              .addMethod(getHashBatchMethod())
              .build();
        }
      }
    }
    return result;
  }
}
