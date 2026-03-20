package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class ExecServiceGrpc {

  private ExecServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.ExecService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecSpawnRequest,
      gradle.substrate.v1.Substrate.ExecSpawnResponse> getSpawnMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "Spawn",
      requestType = gradle.substrate.v1.Substrate.ExecSpawnRequest.class,
      responseType = gradle.substrate.v1.Substrate.ExecSpawnResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecSpawnRequest,
      gradle.substrate.v1.Substrate.ExecSpawnResponse> getSpawnMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecSpawnRequest, gradle.substrate.v1.Substrate.ExecSpawnResponse> getSpawnMethod;
    if ((getSpawnMethod = ExecServiceGrpc.getSpawnMethod) == null) {
      synchronized (ExecServiceGrpc.class) {
        if ((getSpawnMethod = ExecServiceGrpc.getSpawnMethod) == null) {
          ExecServiceGrpc.getSpawnMethod = getSpawnMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ExecSpawnRequest, gradle.substrate.v1.Substrate.ExecSpawnResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "Spawn"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecSpawnRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecSpawnResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecServiceMethodDescriptorSupplier("Spawn"))
              .build();
        }
      }
    }
    return getSpawnMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecWaitRequest,
      gradle.substrate.v1.Substrate.ExecWaitResponse> getWaitMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "Wait",
      requestType = gradle.substrate.v1.Substrate.ExecWaitRequest.class,
      responseType = gradle.substrate.v1.Substrate.ExecWaitResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecWaitRequest,
      gradle.substrate.v1.Substrate.ExecWaitResponse> getWaitMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecWaitRequest, gradle.substrate.v1.Substrate.ExecWaitResponse> getWaitMethod;
    if ((getWaitMethod = ExecServiceGrpc.getWaitMethod) == null) {
      synchronized (ExecServiceGrpc.class) {
        if ((getWaitMethod = ExecServiceGrpc.getWaitMethod) == null) {
          ExecServiceGrpc.getWaitMethod = getWaitMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ExecWaitRequest, gradle.substrate.v1.Substrate.ExecWaitResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "Wait"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecWaitRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecWaitResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecServiceMethodDescriptorSupplier("Wait"))
              .build();
        }
      }
    }
    return getWaitMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecSignalRequest,
      gradle.substrate.v1.Substrate.ExecSignalResponse> getSignalMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "Signal",
      requestType = gradle.substrate.v1.Substrate.ExecSignalRequest.class,
      responseType = gradle.substrate.v1.Substrate.ExecSignalResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecSignalRequest,
      gradle.substrate.v1.Substrate.ExecSignalResponse> getSignalMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecSignalRequest, gradle.substrate.v1.Substrate.ExecSignalResponse> getSignalMethod;
    if ((getSignalMethod = ExecServiceGrpc.getSignalMethod) == null) {
      synchronized (ExecServiceGrpc.class) {
        if ((getSignalMethod = ExecServiceGrpc.getSignalMethod) == null) {
          ExecServiceGrpc.getSignalMethod = getSignalMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ExecSignalRequest, gradle.substrate.v1.Substrate.ExecSignalResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "Signal"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecSignalRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecSignalResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecServiceMethodDescriptorSupplier("Signal"))
              .build();
        }
      }
    }
    return getSignalMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecKillTreeRequest,
      gradle.substrate.v1.Substrate.ExecKillTreeResponse> getKillTreeMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "KillTree",
      requestType = gradle.substrate.v1.Substrate.ExecKillTreeRequest.class,
      responseType = gradle.substrate.v1.Substrate.ExecKillTreeResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecKillTreeRequest,
      gradle.substrate.v1.Substrate.ExecKillTreeResponse> getKillTreeMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecKillTreeRequest, gradle.substrate.v1.Substrate.ExecKillTreeResponse> getKillTreeMethod;
    if ((getKillTreeMethod = ExecServiceGrpc.getKillTreeMethod) == null) {
      synchronized (ExecServiceGrpc.class) {
        if ((getKillTreeMethod = ExecServiceGrpc.getKillTreeMethod) == null) {
          ExecServiceGrpc.getKillTreeMethod = getKillTreeMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ExecKillTreeRequest, gradle.substrate.v1.Substrate.ExecKillTreeResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "KillTree"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecKillTreeRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecKillTreeResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecServiceMethodDescriptorSupplier("KillTree"))
              .build();
        }
      }
    }
    return getKillTreeMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecOutputRequest,
      gradle.substrate.v1.Substrate.ExecOutputChunk> getSubscribeOutputMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "SubscribeOutput",
      requestType = gradle.substrate.v1.Substrate.ExecOutputRequest.class,
      responseType = gradle.substrate.v1.Substrate.ExecOutputChunk.class,
      methodType = io.grpc.MethodDescriptor.MethodType.SERVER_STREAMING)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecOutputRequest,
      gradle.substrate.v1.Substrate.ExecOutputChunk> getSubscribeOutputMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ExecOutputRequest, gradle.substrate.v1.Substrate.ExecOutputChunk> getSubscribeOutputMethod;
    if ((getSubscribeOutputMethod = ExecServiceGrpc.getSubscribeOutputMethod) == null) {
      synchronized (ExecServiceGrpc.class) {
        if ((getSubscribeOutputMethod = ExecServiceGrpc.getSubscribeOutputMethod) == null) {
          ExecServiceGrpc.getSubscribeOutputMethod = getSubscribeOutputMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ExecOutputRequest, gradle.substrate.v1.Substrate.ExecOutputChunk>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.SERVER_STREAMING)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "SubscribeOutput"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecOutputRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ExecOutputChunk.getDefaultInstance()))
              .setSchemaDescriptor(new ExecServiceMethodDescriptorSupplier("SubscribeOutput"))
              .build();
        }
      }
    }
    return getSubscribeOutputMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static ExecServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecServiceStub>() {
        @java.lang.Override
        public ExecServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecServiceStub(channel, callOptions);
        }
      };
    return ExecServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static ExecServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecServiceBlockingV2Stub>() {
        @java.lang.Override
        public ExecServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return ExecServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static ExecServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecServiceBlockingStub>() {
        @java.lang.Override
        public ExecServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecServiceBlockingStub(channel, callOptions);
        }
      };
    return ExecServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static ExecServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecServiceFutureStub>() {
        @java.lang.Override
        public ExecServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecServiceFutureStub(channel, callOptions);
        }
      };
    return ExecServiceFutureStub.newStub(factory, channel);
  }

  /**
   */
  public interface AsyncService {

    /**
     */
    default void spawn(gradle.substrate.v1.Substrate.ExecSpawnRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecSpawnResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getSpawnMethod(), responseObserver);
    }

    /**
     */
    default void wait(gradle.substrate.v1.Substrate.ExecWaitRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecWaitResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getWaitMethod(), responseObserver);
    }

    /**
     */
    default void signal(gradle.substrate.v1.Substrate.ExecSignalRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecSignalResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getSignalMethod(), responseObserver);
    }

    /**
     */
    default void killTree(gradle.substrate.v1.Substrate.ExecKillTreeRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecKillTreeResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getKillTreeMethod(), responseObserver);
    }

    /**
     */
    default void subscribeOutput(gradle.substrate.v1.Substrate.ExecOutputRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecOutputChunk> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getSubscribeOutputMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service ExecService.
   */
  public static abstract class ExecServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return ExecServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service ExecService.
   */
  public static final class ExecServiceStub
      extends io.grpc.stub.AbstractAsyncStub<ExecServiceStub> {
    private ExecServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecServiceStub(channel, callOptions);
    }

    /**
     */
    public void spawn(gradle.substrate.v1.Substrate.ExecSpawnRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecSpawnResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getSpawnMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     */
    public void wait(gradle.substrate.v1.Substrate.ExecWaitRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecWaitResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getWaitMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     */
    public void signal(gradle.substrate.v1.Substrate.ExecSignalRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecSignalResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getSignalMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     */
    public void killTree(gradle.substrate.v1.Substrate.ExecKillTreeRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecKillTreeResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getKillTreeMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     */
    public void subscribeOutput(gradle.substrate.v1.Substrate.ExecOutputRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecOutputChunk> responseObserver) {
      io.grpc.stub.ClientCalls.asyncServerStreamingCall(
          getChannel().newCall(getSubscribeOutputMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service ExecService.
   */
  public static final class ExecServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<ExecServiceBlockingV2Stub> {
    private ExecServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecSpawnResponse spawn(gradle.substrate.v1.Substrate.ExecSpawnRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getSpawnMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecWaitResponse wait(gradle.substrate.v1.Substrate.ExecWaitRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getWaitMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecSignalResponse signal(gradle.substrate.v1.Substrate.ExecSignalRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getSignalMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecKillTreeResponse killTree(gradle.substrate.v1.Substrate.ExecKillTreeRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getKillTreeMethod(), getCallOptions(), request);
    }

    /**
     */
    @io.grpc.ExperimentalApi("https://github.com/grpc/grpc-java/issues/10918")
    public io.grpc.stub.BlockingClientCall<?, gradle.substrate.v1.Substrate.ExecOutputChunk>
        subscribeOutput(gradle.substrate.v1.Substrate.ExecOutputRequest request) {
      return io.grpc.stub.ClientCalls.blockingV2ServerStreamingCall(
          getChannel(), getSubscribeOutputMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service ExecService.
   */
  public static final class ExecServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<ExecServiceBlockingStub> {
    private ExecServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecServiceBlockingStub(channel, callOptions);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecSpawnResponse spawn(gradle.substrate.v1.Substrate.ExecSpawnRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getSpawnMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecWaitResponse wait(gradle.substrate.v1.Substrate.ExecWaitRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getWaitMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecSignalResponse signal(gradle.substrate.v1.Substrate.ExecSignalRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getSignalMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.ExecKillTreeResponse killTree(gradle.substrate.v1.Substrate.ExecKillTreeRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getKillTreeMethod(), getCallOptions(), request);
    }

    /**
     */
    public java.util.Iterator<gradle.substrate.v1.Substrate.ExecOutputChunk> subscribeOutput(
        gradle.substrate.v1.Substrate.ExecOutputRequest request) {
      return io.grpc.stub.ClientCalls.blockingServerStreamingCall(
          getChannel(), getSubscribeOutputMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service ExecService.
   */
  public static final class ExecServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<ExecServiceFutureStub> {
    private ExecServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecServiceFutureStub(channel, callOptions);
    }

    /**
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ExecSpawnResponse> spawn(
        gradle.substrate.v1.Substrate.ExecSpawnRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getSpawnMethod(), getCallOptions()), request);
    }

    /**
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ExecWaitResponse> wait(
        gradle.substrate.v1.Substrate.ExecWaitRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getWaitMethod(), getCallOptions()), request);
    }

    /**
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ExecSignalResponse> signal(
        gradle.substrate.v1.Substrate.ExecSignalRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getSignalMethod(), getCallOptions()), request);
    }

    /**
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ExecKillTreeResponse> killTree(
        gradle.substrate.v1.Substrate.ExecKillTreeRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getKillTreeMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_SPAWN = 0;
  private static final int METHODID_WAIT = 1;
  private static final int METHODID_SIGNAL = 2;
  private static final int METHODID_KILL_TREE = 3;
  private static final int METHODID_SUBSCRIBE_OUTPUT = 4;

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
        case METHODID_SPAWN:
          serviceImpl.spawn((gradle.substrate.v1.Substrate.ExecSpawnRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecSpawnResponse>) responseObserver);
          break;
        case METHODID_WAIT:
          serviceImpl.wait((gradle.substrate.v1.Substrate.ExecWaitRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecWaitResponse>) responseObserver);
          break;
        case METHODID_SIGNAL:
          serviceImpl.signal((gradle.substrate.v1.Substrate.ExecSignalRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecSignalResponse>) responseObserver);
          break;
        case METHODID_KILL_TREE:
          serviceImpl.killTree((gradle.substrate.v1.Substrate.ExecKillTreeRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecKillTreeResponse>) responseObserver);
          break;
        case METHODID_SUBSCRIBE_OUTPUT:
          serviceImpl.subscribeOutput((gradle.substrate.v1.Substrate.ExecOutputRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ExecOutputChunk>) responseObserver);
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
          getSpawnMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ExecSpawnRequest,
              gradle.substrate.v1.Substrate.ExecSpawnResponse>(
                service, METHODID_SPAWN)))
        .addMethod(
          getWaitMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ExecWaitRequest,
              gradle.substrate.v1.Substrate.ExecWaitResponse>(
                service, METHODID_WAIT)))
        .addMethod(
          getSignalMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ExecSignalRequest,
              gradle.substrate.v1.Substrate.ExecSignalResponse>(
                service, METHODID_SIGNAL)))
        .addMethod(
          getKillTreeMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ExecKillTreeRequest,
              gradle.substrate.v1.Substrate.ExecKillTreeResponse>(
                service, METHODID_KILL_TREE)))
        .addMethod(
          getSubscribeOutputMethod(),
          io.grpc.stub.ServerCalls.asyncServerStreamingCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ExecOutputRequest,
              gradle.substrate.v1.Substrate.ExecOutputChunk>(
                service, METHODID_SUBSCRIBE_OUTPUT)))
        .build();
  }

  private static abstract class ExecServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    ExecServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("ExecService");
    }
  }

  private static final class ExecServiceFileDescriptorSupplier
      extends ExecServiceBaseDescriptorSupplier {
    ExecServiceFileDescriptorSupplier() {}
  }

  private static final class ExecServiceMethodDescriptorSupplier
      extends ExecServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    ExecServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (ExecServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new ExecServiceFileDescriptorSupplier())
              .addMethod(getSpawnMethod())
              .addMethod(getWaitMethod())
              .addMethod(getSignalMethod())
              .addMethod(getKillTreeMethod())
              .addMethod(getSubscribeOutputMethod())
              .build();
        }
      }
    }
    return result;
  }
}
