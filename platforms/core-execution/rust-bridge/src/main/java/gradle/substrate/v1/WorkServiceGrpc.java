package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class WorkServiceGrpc {

  private WorkServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.WorkService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.WorkEvaluateRequest,
      gradle.substrate.v1.Substrate.WorkEvaluateResponse> getEvaluateMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "Evaluate",
      requestType = gradle.substrate.v1.Substrate.WorkEvaluateRequest.class,
      responseType = gradle.substrate.v1.Substrate.WorkEvaluateResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.WorkEvaluateRequest,
      gradle.substrate.v1.Substrate.WorkEvaluateResponse> getEvaluateMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.WorkEvaluateRequest, gradle.substrate.v1.Substrate.WorkEvaluateResponse> getEvaluateMethod;
    if ((getEvaluateMethod = WorkServiceGrpc.getEvaluateMethod) == null) {
      synchronized (WorkServiceGrpc.class) {
        if ((getEvaluateMethod = WorkServiceGrpc.getEvaluateMethod) == null) {
          WorkServiceGrpc.getEvaluateMethod = getEvaluateMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.WorkEvaluateRequest, gradle.substrate.v1.Substrate.WorkEvaluateResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "Evaluate"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.WorkEvaluateRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.WorkEvaluateResponse.getDefaultInstance()))
              .setSchemaDescriptor(new WorkServiceMethodDescriptorSupplier("Evaluate"))
              .build();
        }
      }
    }
    return getEvaluateMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.WorkRecordRequest,
      gradle.substrate.v1.Substrate.WorkRecordResponse> getRecordExecutionMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "RecordExecution",
      requestType = gradle.substrate.v1.Substrate.WorkRecordRequest.class,
      responseType = gradle.substrate.v1.Substrate.WorkRecordResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.WorkRecordRequest,
      gradle.substrate.v1.Substrate.WorkRecordResponse> getRecordExecutionMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.WorkRecordRequest, gradle.substrate.v1.Substrate.WorkRecordResponse> getRecordExecutionMethod;
    if ((getRecordExecutionMethod = WorkServiceGrpc.getRecordExecutionMethod) == null) {
      synchronized (WorkServiceGrpc.class) {
        if ((getRecordExecutionMethod = WorkServiceGrpc.getRecordExecutionMethod) == null) {
          WorkServiceGrpc.getRecordExecutionMethod = getRecordExecutionMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.WorkRecordRequest, gradle.substrate.v1.Substrate.WorkRecordResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "RecordExecution"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.WorkRecordRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.WorkRecordResponse.getDefaultInstance()))
              .setSchemaDescriptor(new WorkServiceMethodDescriptorSupplier("RecordExecution"))
              .build();
        }
      }
    }
    return getRecordExecutionMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static WorkServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<WorkServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<WorkServiceStub>() {
        @java.lang.Override
        public WorkServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new WorkServiceStub(channel, callOptions);
        }
      };
    return WorkServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static WorkServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<WorkServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<WorkServiceBlockingV2Stub>() {
        @java.lang.Override
        public WorkServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new WorkServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return WorkServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static WorkServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<WorkServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<WorkServiceBlockingStub>() {
        @java.lang.Override
        public WorkServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new WorkServiceBlockingStub(channel, callOptions);
        }
      };
    return WorkServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static WorkServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<WorkServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<WorkServiceFutureStub>() {
        @java.lang.Override
        public WorkServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new WorkServiceFutureStub(channel, callOptions);
        }
      };
    return WorkServiceFutureStub.newStub(factory, channel);
  }

  /**
   */
  public interface AsyncService {

    /**
     */
    default void evaluate(gradle.substrate.v1.Substrate.WorkEvaluateRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.WorkEvaluateResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getEvaluateMethod(), responseObserver);
    }

    /**
     */
    default void recordExecution(gradle.substrate.v1.Substrate.WorkRecordRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.WorkRecordResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getRecordExecutionMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service WorkService.
   */
  public static abstract class WorkServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return WorkServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service WorkService.
   */
  public static final class WorkServiceStub
      extends io.grpc.stub.AbstractAsyncStub<WorkServiceStub> {
    private WorkServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected WorkServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new WorkServiceStub(channel, callOptions);
    }

    /**
     */
    public void evaluate(gradle.substrate.v1.Substrate.WorkEvaluateRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.WorkEvaluateResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getEvaluateMethod(), getCallOptions()), request, responseObserver);
    }

    /**
     */
    public void recordExecution(gradle.substrate.v1.Substrate.WorkRecordRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.WorkRecordResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getRecordExecutionMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service WorkService.
   */
  public static final class WorkServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<WorkServiceBlockingV2Stub> {
    private WorkServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected WorkServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new WorkServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.WorkEvaluateResponse evaluate(gradle.substrate.v1.Substrate.WorkEvaluateRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getEvaluateMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.WorkRecordResponse recordExecution(gradle.substrate.v1.Substrate.WorkRecordRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getRecordExecutionMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service WorkService.
   */
  public static final class WorkServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<WorkServiceBlockingStub> {
    private WorkServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected WorkServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new WorkServiceBlockingStub(channel, callOptions);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.WorkEvaluateResponse evaluate(gradle.substrate.v1.Substrate.WorkEvaluateRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getEvaluateMethod(), getCallOptions(), request);
    }

    /**
     */
    public gradle.substrate.v1.Substrate.WorkRecordResponse recordExecution(gradle.substrate.v1.Substrate.WorkRecordRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getRecordExecutionMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service WorkService.
   */
  public static final class WorkServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<WorkServiceFutureStub> {
    private WorkServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected WorkServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new WorkServiceFutureStub(channel, callOptions);
    }

    /**
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.WorkEvaluateResponse> evaluate(
        gradle.substrate.v1.Substrate.WorkEvaluateRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getEvaluateMethod(), getCallOptions()), request);
    }

    /**
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.WorkRecordResponse> recordExecution(
        gradle.substrate.v1.Substrate.WorkRecordRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getRecordExecutionMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_EVALUATE = 0;
  private static final int METHODID_RECORD_EXECUTION = 1;

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
        case METHODID_EVALUATE:
          serviceImpl.evaluate((gradle.substrate.v1.Substrate.WorkEvaluateRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.WorkEvaluateResponse>) responseObserver);
          break;
        case METHODID_RECORD_EXECUTION:
          serviceImpl.recordExecution((gradle.substrate.v1.Substrate.WorkRecordRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.WorkRecordResponse>) responseObserver);
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
          getEvaluateMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.WorkEvaluateRequest,
              gradle.substrate.v1.Substrate.WorkEvaluateResponse>(
                service, METHODID_EVALUATE)))
        .addMethod(
          getRecordExecutionMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.WorkRecordRequest,
              gradle.substrate.v1.Substrate.WorkRecordResponse>(
                service, METHODID_RECORD_EXECUTION)))
        .build();
  }

  private static abstract class WorkServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    WorkServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("WorkService");
    }
  }

  private static final class WorkServiceFileDescriptorSupplier
      extends WorkServiceBaseDescriptorSupplier {
    WorkServiceFileDescriptorSupplier() {}
  }

  private static final class WorkServiceMethodDescriptorSupplier
      extends WorkServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    WorkServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (WorkServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new WorkServiceFileDescriptorSupplier())
              .addMethod(getEvaluateMethod())
              .addMethod(getRecordExecutionMethod())
              .build();
        }
      }
    }
    return result;
  }
}
