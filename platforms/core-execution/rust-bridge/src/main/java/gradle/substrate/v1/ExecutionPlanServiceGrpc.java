package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 * <pre>
 * Provides advisory and authoritative execution planning.
 * Phase 5 (advisory): Rust predicts the outcome, Java remains authoritative.
 * Phase 6 (authoritative): Rust drives work identity, caching, and up-to-date decisions.
 * </pre>
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class ExecutionPlanServiceGrpc {

  private ExecutionPlanServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.ExecutionPlanService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.PredictOutcomeRequest,
      gradle.substrate.v1.Substrate.PredictOutcomeResponse> getPredictOutcomeMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "PredictOutcome",
      requestType = gradle.substrate.v1.Substrate.PredictOutcomeRequest.class,
      responseType = gradle.substrate.v1.Substrate.PredictOutcomeResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.PredictOutcomeRequest,
      gradle.substrate.v1.Substrate.PredictOutcomeResponse> getPredictOutcomeMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.PredictOutcomeRequest, gradle.substrate.v1.Substrate.PredictOutcomeResponse> getPredictOutcomeMethod;
    if ((getPredictOutcomeMethod = ExecutionPlanServiceGrpc.getPredictOutcomeMethod) == null) {
      synchronized (ExecutionPlanServiceGrpc.class) {
        if ((getPredictOutcomeMethod = ExecutionPlanServiceGrpc.getPredictOutcomeMethod) == null) {
          ExecutionPlanServiceGrpc.getPredictOutcomeMethod = getPredictOutcomeMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.PredictOutcomeRequest, gradle.substrate.v1.Substrate.PredictOutcomeResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "PredictOutcome"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.PredictOutcomeRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.PredictOutcomeResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecutionPlanServiceMethodDescriptorSupplier("PredictOutcome"))
              .build();
        }
      }
    }
    return getPredictOutcomeMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ResolvePlanRequest,
      gradle.substrate.v1.Substrate.ResolvePlanResponse> getResolvePlanMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "ResolvePlan",
      requestType = gradle.substrate.v1.Substrate.ResolvePlanRequest.class,
      responseType = gradle.substrate.v1.Substrate.ResolvePlanResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ResolvePlanRequest,
      gradle.substrate.v1.Substrate.ResolvePlanResponse> getResolvePlanMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ResolvePlanRequest, gradle.substrate.v1.Substrate.ResolvePlanResponse> getResolvePlanMethod;
    if ((getResolvePlanMethod = ExecutionPlanServiceGrpc.getResolvePlanMethod) == null) {
      synchronized (ExecutionPlanServiceGrpc.class) {
        if ((getResolvePlanMethod = ExecutionPlanServiceGrpc.getResolvePlanMethod) == null) {
          ExecutionPlanServiceGrpc.getResolvePlanMethod = getResolvePlanMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ResolvePlanRequest, gradle.substrate.v1.Substrate.ResolvePlanResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "ResolvePlan"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ResolvePlanRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ResolvePlanResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecutionPlanServiceMethodDescriptorSupplier("ResolvePlan"))
              .build();
        }
      }
    }
    return getResolvePlanMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.RecordOutcomeRequest,
      gradle.substrate.v1.Substrate.RecordOutcomeResponse> getRecordOutcomeMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "RecordOutcome",
      requestType = gradle.substrate.v1.Substrate.RecordOutcomeRequest.class,
      responseType = gradle.substrate.v1.Substrate.RecordOutcomeResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.RecordOutcomeRequest,
      gradle.substrate.v1.Substrate.RecordOutcomeResponse> getRecordOutcomeMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.RecordOutcomeRequest, gradle.substrate.v1.Substrate.RecordOutcomeResponse> getRecordOutcomeMethod;
    if ((getRecordOutcomeMethod = ExecutionPlanServiceGrpc.getRecordOutcomeMethod) == null) {
      synchronized (ExecutionPlanServiceGrpc.class) {
        if ((getRecordOutcomeMethod = ExecutionPlanServiceGrpc.getRecordOutcomeMethod) == null) {
          ExecutionPlanServiceGrpc.getRecordOutcomeMethod = getRecordOutcomeMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.RecordOutcomeRequest, gradle.substrate.v1.Substrate.RecordOutcomeResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "RecordOutcome"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.RecordOutcomeRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.RecordOutcomeResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ExecutionPlanServiceMethodDescriptorSupplier("RecordOutcome"))
              .build();
        }
      }
    }
    return getRecordOutcomeMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static ExecutionPlanServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceStub>() {
        @java.lang.Override
        public ExecutionPlanServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionPlanServiceStub(channel, callOptions);
        }
      };
    return ExecutionPlanServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static ExecutionPlanServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceBlockingV2Stub>() {
        @java.lang.Override
        public ExecutionPlanServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionPlanServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return ExecutionPlanServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static ExecutionPlanServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceBlockingStub>() {
        @java.lang.Override
        public ExecutionPlanServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionPlanServiceBlockingStub(channel, callOptions);
        }
      };
    return ExecutionPlanServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static ExecutionPlanServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ExecutionPlanServiceFutureStub>() {
        @java.lang.Override
        public ExecutionPlanServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ExecutionPlanServiceFutureStub(channel, callOptions);
        }
      };
    return ExecutionPlanServiceFutureStub.newStub(factory, channel);
  }

  /**
   * <pre>
   * Provides advisory and authoritative execution planning.
   * Phase 5 (advisory): Rust predicts the outcome, Java remains authoritative.
   * Phase 6 (authoritative): Rust drives work identity, caching, and up-to-date decisions.
   * </pre>
   */
  public interface AsyncService {

    /**
     * <pre>
     * Phase 5: Predict execution outcome without side effects.
     * </pre>
     */
    default void predictOutcome(gradle.substrate.v1.Substrate.PredictOutcomeRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.PredictOutcomeResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getPredictOutcomeMethod(), responseObserver);
    }

    /**
     * <pre>
     * Phase 6: Resolve the execution plan authoritatively.
     * </pre>
     */
    default void resolvePlan(gradle.substrate.v1.Substrate.ResolvePlanRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ResolvePlanResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getResolvePlanMethod(), responseObserver);
    }

    /**
     * <pre>
     * Record the actual outcome after execution (for shadow mode comparison).
     * </pre>
     */
    default void recordOutcome(gradle.substrate.v1.Substrate.RecordOutcomeRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.RecordOutcomeResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getRecordOutcomeMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service ExecutionPlanService.
   * <pre>
   * Provides advisory and authoritative execution planning.
   * Phase 5 (advisory): Rust predicts the outcome, Java remains authoritative.
   * Phase 6 (authoritative): Rust drives work identity, caching, and up-to-date decisions.
   * </pre>
   */
  public static abstract class ExecutionPlanServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return ExecutionPlanServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service ExecutionPlanService.
   * <pre>
   * Provides advisory and authoritative execution planning.
   * Phase 5 (advisory): Rust predicts the outcome, Java remains authoritative.
   * Phase 6 (authoritative): Rust drives work identity, caching, and up-to-date decisions.
   * </pre>
   */
  public static final class ExecutionPlanServiceStub
      extends io.grpc.stub.AbstractAsyncStub<ExecutionPlanServiceStub> {
    private ExecutionPlanServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionPlanServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionPlanServiceStub(channel, callOptions);
    }

    /**
     * <pre>
     * Phase 5: Predict execution outcome without side effects.
     * </pre>
     */
    public void predictOutcome(gradle.substrate.v1.Substrate.PredictOutcomeRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.PredictOutcomeResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getPredictOutcomeMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     * <pre>
     * Phase 6: Resolve the execution plan authoritatively.
     * </pre>
     */
    public void resolvePlan(gradle.substrate.v1.Substrate.ResolvePlanRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ResolvePlanResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getResolvePlanMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     * <pre>
     * Record the actual outcome after execution (for shadow mode comparison).
     * </pre>
     */
    public void recordOutcome(gradle.substrate.v1.Substrate.RecordOutcomeRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.RecordOutcomeResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getRecordOutcomeMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service ExecutionPlanService.
   * <pre>
   * Provides advisory and authoritative execution planning.
   * Phase 5 (advisory): Rust predicts the outcome, Java remains authoritative.
   * Phase 6 (authoritative): Rust drives work identity, caching, and up-to-date decisions.
   * </pre>
   */
  public static final class ExecutionPlanServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<ExecutionPlanServiceBlockingV2Stub> {
    private ExecutionPlanServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionPlanServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionPlanServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     * <pre>
     * Phase 5: Predict execution outcome without side effects.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.PredictOutcomeResponse predictOutcome(gradle.substrate.v1.Substrate.PredictOutcomeRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getPredictOutcomeMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Phase 6: Resolve the execution plan authoritatively.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.ResolvePlanResponse resolvePlan(gradle.substrate.v1.Substrate.ResolvePlanRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getResolvePlanMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Record the actual outcome after execution (for shadow mode comparison).
     * </pre>
     */
    public gradle.substrate.v1.Substrate.RecordOutcomeResponse recordOutcome(gradle.substrate.v1.Substrate.RecordOutcomeRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getRecordOutcomeMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service ExecutionPlanService.
   * <pre>
   * Provides advisory and authoritative execution planning.
   * Phase 5 (advisory): Rust predicts the outcome, Java remains authoritative.
   * Phase 6 (authoritative): Rust drives work identity, caching, and up-to-date decisions.
   * </pre>
   */
  public static final class ExecutionPlanServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<ExecutionPlanServiceBlockingStub> {
    private ExecutionPlanServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionPlanServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionPlanServiceBlockingStub(channel, callOptions);
    }

    /**
     * <pre>
     * Phase 5: Predict execution outcome without side effects.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.PredictOutcomeResponse predictOutcome(gradle.substrate.v1.Substrate.PredictOutcomeRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getPredictOutcomeMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Phase 6: Resolve the execution plan authoritatively.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.ResolvePlanResponse resolvePlan(gradle.substrate.v1.Substrate.ResolvePlanRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getResolvePlanMethod(), getCallOptions(), request);
    }

    /**
     * <pre>
     * Record the actual outcome after execution (for shadow mode comparison).
     * </pre>
     */
    public gradle.substrate.v1.Substrate.RecordOutcomeResponse recordOutcome(gradle.substrate.v1.Substrate.RecordOutcomeRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getRecordOutcomeMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service ExecutionPlanService.
   * <pre>
   * Provides advisory and authoritative execution planning.
   * Phase 5 (advisory): Rust predicts the outcome, Java remains authoritative.
   * Phase 6 (authoritative): Rust drives work identity, caching, and up-to-date decisions.
   * </pre>
   */
  public static final class ExecutionPlanServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<ExecutionPlanServiceFutureStub> {
    private ExecutionPlanServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ExecutionPlanServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ExecutionPlanServiceFutureStub(channel, callOptions);
    }

    /**
     * <pre>
     * Phase 5: Predict execution outcome without side effects.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.PredictOutcomeResponse> predictOutcome(
        gradle.substrate.v1.Substrate.PredictOutcomeRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getPredictOutcomeMethod(), getCallOptions()), request);
    }

    /**
     * <pre>
     * Phase 6: Resolve the execution plan authoritatively.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ResolvePlanResponse> resolvePlan(
        gradle.substrate.v1.Substrate.ResolvePlanRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getResolvePlanMethod(), getCallOptions()), request);
    }

    /**
     * <pre>
     * Record the actual outcome after execution (for shadow mode comparison).
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.RecordOutcomeResponse> recordOutcome(
        gradle.substrate.v1.Substrate.RecordOutcomeRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getRecordOutcomeMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_PREDICT_OUTCOME = 0;
  private static final int METHODID_RESOLVE_PLAN = 1;
  private static final int METHODID_RECORD_OUTCOME = 2;

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
        case METHODID_PREDICT_OUTCOME:
          serviceImpl.predictOutcome((gradle.substrate.v1.Substrate.PredictOutcomeRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.PredictOutcomeResponse>) responseObserver);
          break;
        case METHODID_RESOLVE_PLAN:
          serviceImpl.resolvePlan((gradle.substrate.v1.Substrate.ResolvePlanRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ResolvePlanResponse>) responseObserver);
          break;
        case METHODID_RECORD_OUTCOME:
          serviceImpl.recordOutcome((gradle.substrate.v1.Substrate.RecordOutcomeRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.RecordOutcomeResponse>) responseObserver);
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
          getPredictOutcomeMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.PredictOutcomeRequest,
              gradle.substrate.v1.Substrate.PredictOutcomeResponse>(
                service, METHODID_PREDICT_OUTCOME)))
        .addMethod(
          getResolvePlanMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ResolvePlanRequest,
              gradle.substrate.v1.Substrate.ResolvePlanResponse>(
                service, METHODID_RESOLVE_PLAN)))
        .addMethod(
          getRecordOutcomeMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.RecordOutcomeRequest,
              gradle.substrate.v1.Substrate.RecordOutcomeResponse>(
                service, METHODID_RECORD_OUTCOME)))
        .build();
  }

  private static abstract class ExecutionPlanServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    ExecutionPlanServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("ExecutionPlanService");
    }
  }

  private static final class ExecutionPlanServiceFileDescriptorSupplier
      extends ExecutionPlanServiceBaseDescriptorSupplier {
    ExecutionPlanServiceFileDescriptorSupplier() {}
  }

  private static final class ExecutionPlanServiceMethodDescriptorSupplier
      extends ExecutionPlanServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    ExecutionPlanServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (ExecutionPlanServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new ExecutionPlanServiceFileDescriptorSupplier())
              .addMethod(getPredictOutcomeMethod())
              .addMethod(getResolvePlanMethod())
              .addMethod(getRecordOutcomeMethod())
              .build();
        }
      }
    }
    return result;
  }
}
