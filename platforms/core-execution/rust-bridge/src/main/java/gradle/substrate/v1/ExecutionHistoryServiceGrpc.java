package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 * <pre>
 * Stores and retrieves execution history for up-to-date checking.
 * Replaces Java's ExecutionHistoryStore with Rust-backed storage.
 * </pre>
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class ExecutionHistoryServiceGrpc {

  private ExecutionHistoryServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.ExecutionHistoryService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.LoadHistoryRequest,
      gradle.substrate.v1.Substrate.LoadHistoryResponse> getLoadHistoryMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "LoadHistory",
      requestType = gradle.substrate.v1.Substrate.LoadHistoryRequest.class,
      responseType = gradle.substrate.v1.Substrate.LoadHistoryResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.LoadHistoryRequest,
      gradle.substrate.v1.Substrate.LoadHistoryResponse> getLoadHistoryMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.LoadHistoryRequest, gradle.substrate.v1.Substrate.LoadHistoryResponse> getLoadHistoryMethod;
    if ((getLoadHistoryMethod = ExecutionHistoryServiceGrpc.getLoadHistoryMethod) == null) {
      synchronized (ExecutionHistoryServiceGrpc.class) {
        if ((getLoadHistoryMethod = ExecutionHistoryServiceGrpc.getLoadHistoryMethod) == null) {
          ExecutionHistoryServiceGrpc.getLoadHistoryMethod = getLoadHistoryMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.LoadHistoryRequest, gradle.substrate.v1.Substrate.LoadHistoryResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "LoadHistory"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.LoadHistoryRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.LoadHistoryResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecutionHistoryServiceMethodDescriptorSupplier("LoadHistory"))
              .build();
        }
      }
    }
    return getLoadHistoryMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.StoreHistoryRequest,
      gradle.substrate.v1.Substrate.StoreHistoryResponse> getStoreHistoryMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "StoreHistory",
      requestType = gradle.substrate.v1.Substrate.StoreHistoryRequest.class,
      responseType = gradle.substrate.v1.Substrate.StoreHistoryResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.StoreHistoryRequest,
      gradle.substrate.v1.Substrate.StoreHistoryResponse> getStoreHistoryMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.StoreHistoryRequest, gradle.substrate.v1.Substrate.StoreHistoryResponse> getStoreHistoryMethod;
    if ((getStoreHistoryMethod = ExecutionHistoryServiceGrpc.getStoreHistoryMethod) == null) {
      synchronized (ExecutionHistoryServiceGrpc.class) {
        if ((getStoreHistoryMethod = ExecutionHistoryServiceGrpc.getStoreHistoryMethod) == null) {
          ExecutionHistoryServiceGrpc.getStoreHistoryMethod = getStoreHistoryMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.StoreHistoryRequest, gradle.substrate.v1.Substrate.StoreHistoryResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "StoreHistory"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.StoreHistoryRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.StoreHistoryResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecutionHistoryServiceMethodDescriptorSupplier("StoreHistory"))
              .build();
        }
      }
    }
    return getStoreHistoryMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.RemoveHistoryRequest,
      gradle.substrate.v1.Substrate.RemoveHistoryResponse> getRemoveHistoryMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "RemoveHistory",
      requestType = gradle.substrate.v1.Substrate.RemoveHistoryRequest.class,
      responseType = gradle.substrate.v1.Substrate.RemoveHistoryResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.RemoveHistoryRequest,
      gradle.substrate.v1.Substrate.RemoveHistoryResponse> getRemoveHistoryMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.RemoveHistoryRequest, gradle.substrate.v1.Substrate.RemoveHistoryResponse> getRemoveHistoryMethod;
    if ((getRemoveHistoryMethod = ExecutionHistoryServiceGrpc.getRemoveHistoryMethod) == null) {
      synchronized (ExecutionHistoryServiceGrpc.class) {
        if ((getRemoveHistoryMethod = ExecutionHistoryServiceGrpc.getRemoveHistoryMethod) == null) {
          ExecutionHistoryServiceGrpc.getRemoveHistoryMethod = getRemoveHistoryMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.RemoveHistoryRequest, gradle.substrate.v1.Substrate.RemoveHistoryResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "RemoveHistory"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.RemoveHistoryRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.RemoveHistoryResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecutionHistoryServiceMethodDescriptorSupplier("RemoveHistory"))
              .build();
        }
      }
    }
    return getRemoveHistoryMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static ExecutionHistoryServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceStub>() {
        @java.lang.Override
        public ExecutionHistoryServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionHistoryServiceStub(channel, callOptions);
        }
      };
    return ExecutionHistoryServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static ExecutionHistoryServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceBlockingV2Stub>() {
        @java.lang.Override
        public ExecutionHistoryServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionHistoryServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return ExecutionHistoryServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static ExecutionHistoryServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceBlockingStub>() {
        @java.lang.Override
        public ExecutionHistoryServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionHistoryServiceBlockingStub(channel, callOptions);
        }
      };
    return ExecutionHistoryServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static ExecutionHistoryServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionHistoryServiceFutureStub>() {
        @java.lang.Override
        public ExecutionHistoryServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionHistoryServiceFutureStub(channel, callOptions);
        }
      };
    return ExecutionHistoryServiceFutureStub.newStub(factory, channel);
  }

  /**
   * <pre>
   * Stores and retrieves execution history for up-to-date checking.
   * Replaces Java's ExecutionHistoryStore with Rust-backed storage.
   * </pre>
   */
  public interface AsyncService {

    /**
     * <pre>
     * Load previous execution state for a work unit.
     * </pre>
     */
    default void loadHistory(gradle.substrate.v1.Substrate.LoadHistoryRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.LoadHistoryResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getLoadHistoryMethod(), responseObserver);
    }

    /**
     * <pre>
     * Store execution state after execution completes.
     * </pre>
     */
    default void storeHistory(gradle.substrate.v1.Substrate.StoreHistoryRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.StoreHistoryResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getStoreHistoryMethod(), responseObserver);
    }

    /**
     * <pre>
     * Remove execution history for a work unit.
     * </pre>
     */
    default void removeHistory(gradle.substrate.v1.Substrate.RemoveHistoryRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.RemoveHistoryResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getRemoveHistoryMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service ExecutionHistoryService.
   * <pre>
   * Stores and retrieves execution history for up-to-date checking.
   * Replaces Java's ExecutionHistoryStore with Rust-backed storage.
   * </pre>
   */
  public static abstract class ExecutionHistoryServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return ExecutionHistoryServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service ExecutionHistoryService.
   * <pre>
   * Stores and retrieves execution history for up-to-date checking.
   * Replaces Java's ExecutionHistoryStore with Rust-backed storage.
   * </pre>
   */
  public static final class ExecutionHistoryServiceStub
      extends io.grpc.stub.AbstractAsyncStub<ExecutionHistoryServiceStub> {
    private ExecutionHistoryServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionHistoryServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionHistoryServiceStub(channel, callOptions);
    }

    /**
     * <pre>
     * Load previous execution state for a work unit.
     * </pre>
     */
    public void loadHistory(gradle.substrate.v1.Substrate.LoadHistoryRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.LoadHistoryResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getLoadHistoryMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     * <pre>
     * Store execution state after execution completes.
     * </pre>
     */
    public void storeHistory(gradle.substrate.v1.Substrate.StoreHistoryRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.StoreHistoryResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getStoreHistoryMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     * <pre>
     * Remove execution history for a work unit.
     * </pre>
     */
    public void removeHistory(gradle.substrate.v1.Substrate.RemoveHistoryRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.RemoveHistoryResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getRemoveHistoryMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service ExecutionHistoryService.
   * <pre>
   * Stores and retrieves execution history for up-to-date checking.
   * Replaces Java's ExecutionHistoryStore with Rust-backed storage.
   * </pre>
   */
  public static final class ExecutionHistoryServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<ExecutionHistoryServiceBlockingV2Stub> {
    private ExecutionHistoryServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionHistoryServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionHistoryServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     * <pre>
     * Load previous execution state for a work unit.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.LoadHistoryResponse loadHistory(gradle.substrate.v1.Substrate.LoadHistoryRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getLoadHistoryMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Store execution state after execution completes.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.StoreHistoryResponse storeHistory(gradle.substrate.v1.Substrate.StoreHistoryRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getStoreHistoryMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Remove execution history for a work unit.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.RemoveHistoryResponse removeHistory(gradle.substrate.v1.Substrate.RemoveHistoryRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getRemoveHistoryMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service ExecutionHistoryService.
   * <pre>
   * Stores and retrieves execution history for up-to-date checking.
   * Replaces Java's ExecutionHistoryStore with Rust-backed storage.
   * </pre>
   */
  public static final class ExecutionHistoryServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<ExecutionHistoryServiceBlockingStub> {
    private ExecutionHistoryServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionHistoryServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionHistoryServiceBlockingStub(channel, callOptions);
    }

    /**
     * <pre>
     * Load previous execution state for a work unit.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.LoadHistoryResponse loadHistory(gradle.substrate.v1.Substrate.LoadHistoryRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getLoadHistoryMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Store execution state after execution completes.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.StoreHistoryResponse storeHistory(gradle.substrate.v1.Substrate.StoreHistoryRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getStoreHistoryMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Remove execution history for a work unit.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.RemoveHistoryResponse removeHistory(gradle.substrate.v1.Substrate.RemoveHistoryRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getRemoveHistoryMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service ExecutionHistoryService.
   * <pre>
   * Stores and retrieves execution history for up-to-date checking.
   * Replaces Java's ExecutionHistoryStore with Rust-backed storage.
   * </pre>
   */
  public static final class ExecutionHistoryServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<ExecutionHistoryServiceFutureStub> {
    private ExecutionHistoryServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionHistoryServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionHistoryServiceFutureStub(channel, callOptions);
    }

    /**
     * <pre>
     * Load previous execution state for a work unit.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.LoadHistoryResponse> loadHistory(
        gradle.substrate.v1.Substrate.LoadHistoryRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getLoadHistoryMethod(), getCallOptions()), request);
    }

    /**
     * <pre>
     * Store execution state after execution completes.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.StoreHistoryResponse> storeHistory(
        gradle.substrate.v1.Substrate.StoreHistoryRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getStoreHistoryMethod(), getCallOptions()), request);
    }

    /**
     * <pre>
     * Remove execution history for a work unit.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.RemoveHistoryResponse> removeHistory(
        gradle.substrate.v1.Substrate.RemoveHistoryRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getRemoveHistoryMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_LOAD_HISTORY = 0;
  private static final int METHODID_STORE_HISTORY = 1;
  private static final int METHODID_REMOVE_HISTORY = 2;

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
        case METHODID_LOAD_HISTORY:
          serviceImpl.loadHistory((gradle.substrate.v1.Substrate.LoadHistoryRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.LoadHistoryResponse>) responseObserver);
          break;
        case METHODID_STORE_HISTORY:
          serviceImpl.storeHistory((gradle.substrate.v1.Substrate.StoreHistoryRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.StoreHistoryResponse>) responseObserver);
          break;
        case METHODID_REMOVE_HISTORY:
          serviceImpl.removeHistory((gradle.substrate.v1.Substrate.RemoveHistoryRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.RemoveHistoryResponse>) responseObserver);
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
          getLoadHistoryMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.LoadHistoryRequest,
              gradle.substrate.v1.Substrate.LoadHistoryResponse>(
                service, METHODID_LOAD_HISTORY)))
        .addMethod(
          getStoreHistoryMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.StoreHistoryRequest,
              gradle.substrate.v1.Substrate.StoreHistoryResponse>(
                service, METHODID_STORE_HISTORY)))
        .addMethod(
          getRemoveHistoryMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.RemoveHistoryRequest,
              gradle.substrate.v1.Substrate.RemoveHistoryResponse>(
                service, METHODID_REMOVE_HISTORY)))
        .build();
  }

  private static abstract class ExecutionHistoryServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    ExecutionHistoryServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("ExecutionHistoryService");
    }
  }

  private static final class ExecutionHistoryServiceFileDescriptorSupplier
      extends ExecutionHistoryServiceBaseDescriptorSupplier {
    ExecutionHistoryServiceFileDescriptorSupplier() {}
  }

  private static final class ExecutionHistoryServiceMethodDescriptorSupplier
      extends ExecutionHistoryServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    ExecutionHistoryServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (ExecutionHistoryServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new ExecutionHistoryServiceFileDescriptorSupplier())
              .addMethod(getLoadHistoryMethod())
              .addMethod(getStoreHistoryMethod())
              .addMethod(getRemoveHistoryMethod())
              .build();
        }
      }
    }
    return result;
  }
}
