package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 * <pre>
 * Coordinates build cache operations from Rust.
 * Replaces Java's BuildCacheController with Rust-backed orchestration.
 * </pre>
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class BuildCacheOrchestrationServiceGrpc {

  private BuildCacheOrchestrationServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.BuildCacheOrchestrationService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ComputeCacheKeyRequest,
      gradle.substrate.v1.Substrate.ComputeCacheKeyResponse> getComputeCacheKeyMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "ComputeCacheKey",
      requestType = gradle.substrate.v1.Substrate.ComputeCacheKeyRequest.class,
      responseType = gradle.substrate.v1.Substrate.ComputeCacheKeyResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ComputeCacheKeyRequest,
      gradle.substrate.v1.Substrate.ComputeCacheKeyResponse> getComputeCacheKeyMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ComputeCacheKeyRequest, gradle.substrate.v1.Substrate.ComputeCacheKeyResponse> getComputeCacheKeyMethod;
    if ((getComputeCacheKeyMethod = BuildCacheOrchestrationServiceGrpc.getComputeCacheKeyMethod) == null) {
      synchronized (BuildCacheOrchestrationServiceGrpc.class) {
        if ((getComputeCacheKeyMethod = BuildCacheOrchestrationServiceGrpc.getComputeCacheKeyMethod) == null) {
          BuildCacheOrchestrationServiceGrpc.getComputeCacheKeyMethod = getComputeCacheKeyMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ComputeCacheKeyRequest, gradle.substrate.v1.Substrate.ComputeCacheKeyResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "ComputeCacheKey"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ComputeCacheKeyRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ComputeCacheKeyResponse.getDefaultInstance()))
              .setSchemaDescriptor(new BuildCacheOrchestrationServiceMethodDescriptorSupplier("ComputeCacheKey"))
              .build();
        }
      }
    }
    return getComputeCacheKeyMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ProbeCacheRequest,
      gradle.substrate.v1.Substrate.ProbeCacheResponse> getProbeCacheMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "ProbeCache",
      requestType = gradle.substrate.v1.Substrate.ProbeCacheRequest.class,
      responseType = gradle.substrate.v1.Substrate.ProbeCacheResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ProbeCacheRequest,
      gradle.substrate.v1.Substrate.ProbeCacheResponse> getProbeCacheMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ProbeCacheRequest, gradle.substrate.v1.Substrate.ProbeCacheResponse> getProbeCacheMethod;
    if ((getProbeCacheMethod = BuildCacheOrchestrationServiceGrpc.getProbeCacheMethod) == null) {
      synchronized (BuildCacheOrchestrationServiceGrpc.class) {
        if ((getProbeCacheMethod = BuildCacheOrchestrationServiceGrpc.getProbeCacheMethod) == null) {
          BuildCacheOrchestrationServiceGrpc.getProbeCacheMethod = getProbeCacheMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ProbeCacheRequest, gradle.substrate.v1.Substrate.ProbeCacheResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "ProbeCache"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ProbeCacheRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ProbeCacheResponse.getDefaultInstance()))
              .setSchemaDescriptor(new BuildCacheOrchestrationServiceMethodDescriptorSupplier("ProbeCache"))
              .build();
        }
      }
    }
    return getProbeCacheMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.StoreOutputsRequest,
      gradle.substrate.v1.Substrate.StoreOutputsResponse> getStoreOutputsMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "StoreOutputs",
      requestType = gradle.substrate.v1.Substrate.StoreOutputsRequest.class,
      responseType = gradle.substrate.v1.Substrate.StoreOutputsResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.StoreOutputsRequest,
      gradle.substrate.v1.Substrate.StoreOutputsResponse> getStoreOutputsMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.StoreOutputsRequest, gradle.substrate.v1.Substrate.StoreOutputsResponse> getStoreOutputsMethod;
    if ((getStoreOutputsMethod = BuildCacheOrchestrationServiceGrpc.getStoreOutputsMethod) == null) {
      synchronized (BuildCacheOrchestrationServiceGrpc.class) {
        if ((getStoreOutputsMethod = BuildCacheOrchestrationServiceGrpc.getStoreOutputsMethod) == null) {
          BuildCacheOrchestrationServiceGrpc.getStoreOutputsMethod = getStoreOutputsMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.StoreOutputsRequest, gradle.substrate.v1.Substrate.StoreOutputsResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "StoreOutputs"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.StoreOutputsRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.StoreOutputsResponse.getDefaultInstance()))
              .setSchemaDescriptor(new BuildCacheOrchestrationServiceMethodDescriptorSupplier("StoreOutputs"))
              .build();
        }
      }
    }
    return getStoreOutputsMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static BuildCacheOrchestrationServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceStub>() {
        @java.lang.Override
        public BuildCacheOrchestrationServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new BuildCacheOrchestrationServiceStub(channel, callOptions);
        }
      };
    return BuildCacheOrchestrationServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static BuildCacheOrchestrationServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceBlockingV2Stub>() {
        @java.lang.Override
        public BuildCacheOrchestrationServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new BuildCacheOrchestrationServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return BuildCacheOrchestrationServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static BuildCacheOrchestrationServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceBlockingStub>() {
        @java.lang.Override
        public BuildCacheOrchestrationServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new BuildCacheOrchestrationServiceBlockingStub(channel, callOptions);
        }
      };
    return BuildCacheOrchestrationServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static BuildCacheOrchestrationServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<BuildCacheOrchestrationServiceFutureStub>() {
        @java.lang.Override
        public BuildCacheOrchestrationServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new BuildCacheOrchestrationServiceFutureStub(channel, callOptions);
        }
      };
    return BuildCacheOrchestrationServiceFutureStub.newStub(factory, channel);
  }

  /**
   * <pre>
   * Coordinates build cache operations from Rust.
   * Replaces Java's BuildCacheController with Rust-backed orchestration.
   * </pre>
   */
  public interface AsyncService {

    /**
     * <pre>
     * Compute a cache key for the given work identity and input fingerprints.
     * </pre>
     */
    default void computeCacheKey(gradle.substrate.v1.Substrate.ComputeCacheKeyRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ComputeCacheKeyResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getComputeCacheKeyMethod(), responseObserver);
    }

    /**
     * <pre>
     * Check if a cache entry exists without loading it.
     * </pre>
     */
    default void probeCache(gradle.substrate.v1.Substrate.ProbeCacheRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ProbeCacheResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getProbeCacheMethod(), responseObserver);
    }

    /**
     * <pre>
     * Pack execution outputs into a cache entry (delegates to existing CacheService).
     * </pre>
     */
    default void storeOutputs(gradle.substrate.v1.Substrate.StoreOutputsRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.StoreOutputsResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getStoreOutputsMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service BuildCacheOrchestrationService.
   * <pre>
   * Coordinates build cache operations from Rust.
   * Replaces Java's BuildCacheController with Rust-backed orchestration.
   * </pre>
   */
  public static abstract class BuildCacheOrchestrationServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return BuildCacheOrchestrationServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service BuildCacheOrchestrationService.
   * <pre>
   * Coordinates build cache operations from Rust.
   * Replaces Java's BuildCacheController with Rust-backed orchestration.
   * </pre>
   */
  public static final class BuildCacheOrchestrationServiceStub
      extends io.grpc.stub.AbstractAsyncStub<BuildCacheOrchestrationServiceStub> {
    private BuildCacheOrchestrationServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected BuildCacheOrchestrationServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new BuildCacheOrchestrationServiceStub(channel, callOptions);
    }

    /**
     * <pre>
     * Compute a cache key for the given work identity and input fingerprints.
     * </pre>
     */
    public void computeCacheKey(gradle.substrate.v1.Substrate.ComputeCacheKeyRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ComputeCacheKeyResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getComputeCacheKeyMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     * <pre>
     * Check if a cache entry exists without loading it.
     * </pre>
     */
    public void probeCache(gradle.substrate.v1.Substrate.ProbeCacheRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ProbeCacheResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getProbeCacheMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     * <pre>
     * Pack execution outputs into a cache entry (delegates to existing CacheService).
     * </pre>
     */
    public void storeOutputs(gradle.substrate.v1.Substrate.StoreOutputsRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.StoreOutputsResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getStoreOutputsMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service BuildCacheOrchestrationService.
   * <pre>
   * Coordinates build cache operations from Rust.
   * Replaces Java's BuildCacheController with Rust-backed orchestration.
   * </pre>
   */
  public static final class BuildCacheOrchestrationServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<BuildCacheOrchestrationServiceBlockingV2Stub> {
    private BuildCacheOrchestrationServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected BuildCacheOrchestrationServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new BuildCacheOrchestrationServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     * <pre>
     * Compute a cache key for the given work identity and input fingerprints.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.ComputeCacheKeyResponse computeCacheKey(gradle.substrate.v1.Substrate.ComputeCacheKeyRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getComputeCacheKeyMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Check if a cache entry exists without loading it.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.ProbeCacheResponse probeCache(gradle.substrate.v1.Substrate.ProbeCacheRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getProbeCacheMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Pack execution outputs into a cache entry (delegates to existing CacheService).
     * </pre>
     */
    public gradle.substrate.v1.Substrate.StoreOutputsResponse storeOutputs(gradle.substrate.v1.Substrate.StoreOutputsRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getStoreOutputsMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service BuildCacheOrchestrationService.
   * <pre>
   * Coordinates build cache operations from Rust.
   * Replaces Java's BuildCacheController with Rust-backed orchestration.
   * </pre>
   */
  public static final class BuildCacheOrchestrationServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<BuildCacheOrchestrationServiceBlockingStub> {
    private BuildCacheOrchestrationServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected BuildCacheOrchestrationServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new BuildCacheOrchestrationServiceBlockingStub(channel, callOptions);
    }

    /**
     * <pre>
     * Compute a cache key for the given work identity and input fingerprints.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.ComputeCacheKeyResponse computeCacheKey(gradle.substrate.v1.Substrate.ComputeCacheKeyRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getComputeCacheKeyMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Check if a cache entry exists without loading it.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.ProbeCacheResponse probeCache(gradle.substrate.v1.Substrate.ProbeCacheRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getProbeCacheMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Pack execution outputs into a cache entry (delegates to existing CacheService).
     * </pre>
     */
    public gradle.substrate.v1.Substrate.StoreOutputsResponse storeOutputs(gradle.substrate.v1.Substrate.StoreOutputsRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getStoreOutputsMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service BuildCacheOrchestrationService.
   * <pre>
   * Coordinates build cache operations from Rust.
   * Replaces Java's BuildCacheController with Rust-backed orchestration.
   * </pre>
   */
  public static final class BuildCacheOrchestrationServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<BuildCacheOrchestrationServiceFutureStub> {
    private BuildCacheOrchestrationServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected BuildCacheOrchestrationServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new BuildCacheOrchestrationServiceFutureStub(channel, callOptions);
    }

    /**
     * <pre>
     * Compute a cache key for the given work identity and input fingerprints.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ComputeCacheKeyResponse> computeCacheKey(
        gradle.substrate.v1.Substrate.ComputeCacheKeyRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getComputeCacheKeyMethod(), getCallOptions()), request);
    }

    /**
     * <pre>
     * Check if a cache entry exists without loading it.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ProbeCacheResponse> probeCache(
        gradle.substrate.v1.Substrate.ProbeCacheRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getProbeCacheMethod(), getCallOptions()), request);
    }

    /**
     * <pre>
     * Pack execution outputs into a cache entry (delegates to existing CacheService).
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.StoreOutputsResponse> storeOutputs(
        gradle.substrate.v1.Substrate.StoreOutputsRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getStoreOutputsMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_COMPUTE_CACHE_KEY = 0;
  private static final int METHODID_PROBE_CACHE = 1;
  private static final int METHODID_STORE_OUTPUTS = 2;

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
        case METHODID_COMPUTE_CACHE_KEY:
          serviceImpl.computeCacheKey((gradle.substrate.v1.Substrate.ComputeCacheKeyRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ComputeCacheKeyResponse>) responseObserver);
          break;
        case METHODID_PROBE_CACHE:
          serviceImpl.probeCache((gradle.substrate.v1.Substrate.ProbeCacheRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ProbeCacheResponse>) responseObserver);
          break;
        case METHODID_STORE_OUTPUTS:
          serviceImpl.storeOutputs((gradle.substrate.v1.Substrate.StoreOutputsRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.StoreOutputsResponse>) responseObserver);
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
          getComputeCacheKeyMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ComputeCacheKeyRequest,
              gradle.substrate.v1.Substrate.ComputeCacheKeyResponse>(
                service, METHODID_COMPUTE_CACHE_KEY)))
        .addMethod(
          getProbeCacheMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ProbeCacheRequest,
              gradle.substrate.v1.Substrate.ProbeCacheResponse>(
                service, METHODID_PROBE_CACHE)))
        .addMethod(
          getStoreOutputsMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.StoreOutputsRequest,
              gradle.substrate.v1.Substrate.StoreOutputsResponse>(
                service, METHODID_STORE_OUTPUTS)))
        .build();
  }

  private static abstract class BuildCacheOrchestrationServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    BuildCacheOrchestrationServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("BuildCacheOrchestrationService");
    }
  }

  private static final class BuildCacheOrchestrationServiceFileDescriptorSupplier
      extends BuildCacheOrchestrationServiceBaseDescriptorSupplier {
    BuildCacheOrchestrationServiceFileDescriptorSupplier() {}
  }

  private static final class BuildCacheOrchestrationServiceMethodDescriptorSupplier
      extends BuildCacheOrchestrationServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    BuildCacheOrchestrationServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (BuildCacheOrchestrationServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new BuildCacheOrchestrationServiceFileDescriptorSupplier())
              .addMethod(getComputeCacheKeyMethod())
              .addMethod(getProbeCacheMethod())
              .addMethod(getStoreOutputsMethod())
              .build();
        }
      }
    }
    return result;
  }
}
