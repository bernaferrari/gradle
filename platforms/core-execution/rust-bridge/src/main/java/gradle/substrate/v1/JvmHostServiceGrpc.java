package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class JvmHostServiceGrpc {

  private JvmHostServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.JvmHostService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.EvaluateScriptRequest,
      gradle.substrate.v1.Substrate.EvaluateScriptResponse> getEvaluateScriptMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "EvaluateScript",
      requestType = gradle.substrate.v1.Substrate.EvaluateScriptRequest.class,
      responseType = gradle.substrate.v1.Substrate.EvaluateScriptResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.EvaluateScriptRequest,
      gradle.substrate.v1.Substrate.EvaluateScriptResponse> getEvaluateScriptMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.EvaluateScriptRequest, gradle.substrate.v1.Substrate.EvaluateScriptResponse> getEvaluateScriptMethod;
    if ((getEvaluateScriptMethod = JvmHostServiceGrpc.getEvaluateScriptMethod) == null) {
      synchronized (JvmHostServiceGrpc.class) {
        if ((getEvaluateScriptMethod = JvmHostServiceGrpc.getEvaluateScriptMethod) == null) {
          JvmHostServiceGrpc.getEvaluateScriptMethod = getEvaluateScriptMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.EvaluateScriptRequest, gradle.substrate.v1.Substrate.EvaluateScriptResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "EvaluateScript"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.EvaluateScriptRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.EvaluateScriptResponse.getDefaultInstance()))
              .setSchemaDescriptor(new JvmHostServiceMethodDescriptorSupplier("EvaluateScript"))
              .build();
        }
      }
    }
    return getEvaluateScriptMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.GetBuildModelRequest,
      gradle.substrate.v1.Substrate.GetBuildModelResponse> getGetBuildModelMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "GetBuildModel",
      requestType = gradle.substrate.v1.Substrate.GetBuildModelRequest.class,
      responseType = gradle.substrate.v1.Substrate.GetBuildModelResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.GetBuildModelRequest,
      gradle.substrate.v1.Substrate.GetBuildModelResponse> getGetBuildModelMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.GetBuildModelRequest, gradle.substrate.v1.Substrate.GetBuildModelResponse> getGetBuildModelMethod;
    if ((getGetBuildModelMethod = JvmHostServiceGrpc.getGetBuildModelMethod) == null) {
      synchronized (JvmHostServiceGrpc.class) {
        if ((getGetBuildModelMethod = JvmHostServiceGrpc.getGetBuildModelMethod) == null) {
          JvmHostServiceGrpc.getGetBuildModelMethod = getGetBuildModelMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.GetBuildModelRequest, gradle.substrate.v1.Substrate.GetBuildModelResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "GetBuildModel"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.GetBuildModelRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.GetBuildModelResponse.getDefaultInstance()))
              .setSchemaDescriptor(new JvmHostServiceMethodDescriptorSupplier("GetBuildModel"))
              .build();
        }
      }
    }
    return getGetBuildModelMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ResolveConfigRequest,
      gradle.substrate.v1.Substrate.ResolveConfigResponse> getResolveConfigurationMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "ResolveConfiguration",
      requestType = gradle.substrate.v1.Substrate.ResolveConfigRequest.class,
      responseType = gradle.substrate.v1.Substrate.ResolveConfigResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ResolveConfigRequest,
      gradle.substrate.v1.Substrate.ResolveConfigResponse> getResolveConfigurationMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.ResolveConfigRequest, gradle.substrate.v1.Substrate.ResolveConfigResponse> getResolveConfigurationMethod;
    if ((getResolveConfigurationMethod = JvmHostServiceGrpc.getResolveConfigurationMethod) == null) {
      synchronized (JvmHostServiceGrpc.class) {
        if ((getResolveConfigurationMethod = JvmHostServiceGrpc.getResolveConfigurationMethod) == null) {
          JvmHostServiceGrpc.getResolveConfigurationMethod = getResolveConfigurationMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.ResolveConfigRequest, gradle.substrate.v1.Substrate.ResolveConfigResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "ResolveConfiguration"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ResolveConfigRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.ResolveConfigResponse.getDefaultInstance()))
              .setSchemaDescriptor(new JvmHostServiceMethodDescriptorSupplier("ResolveConfiguration"))
              .build();
        }
      }
    }
    return getResolveConfigurationMethod;
  }

  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest,
      gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse> getGetBuildEnvironmentMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "GetBuildEnvironment",
      requestType = gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest.class,
      responseType = gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest,
      gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse> getGetBuildEnvironmentMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest, gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse> getGetBuildEnvironmentMethod;
    if ((getGetBuildEnvironmentMethod = JvmHostServiceGrpc.getGetBuildEnvironmentMethod) == null) {
      synchronized (JvmHostServiceGrpc.class) {
        if ((getGetBuildEnvironmentMethod = JvmHostServiceGrpc.getGetBuildEnvironmentMethod) == null) {
          JvmHostServiceGrpc.getGetBuildEnvironmentMethod = getGetBuildEnvironmentMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest, gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "GetBuildEnvironment"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse.getDefaultInstance()))
              .setSchemaDescriptor(new JvmHostServiceMethodDescriptorSupplier("GetBuildEnvironment"))
              .build();
        }
      }
    }
    return getGetBuildEnvironmentMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static JvmHostServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceStub>() {
        @java.lang.Override
        public JvmHostServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new JvmHostServiceStub(channel, callOptions);
        }
      };
    return JvmHostServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static JvmHostServiceBlockingV2Stub newBlockingV2Stub(
    io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceBlockingV2Stub>() {
        @java.lang.Override
        public JvmHostServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new JvmHostServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return JvmHostServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static JvmHostServiceBlockingStub newBlockingStub(
    io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceBlockingStub>() {
        @java.lang.Override
        public JvmHostServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new JvmHostServiceBlockingStub(channel, callOptions);
        }
      };
    return JvmHostServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static JvmHostServiceFutureStub newFutureStub(
    io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<JvmHostServiceFutureStub>() {
        @java.lang.Override
        public JvmHostServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new JvmHostServiceFutureStub(channel, callOptions);
        }
      };
    return JvmHostServiceFutureStub.newStub(factory, channel);
  }

  /**
   */
  public interface AsyncService {

    default void evaluateScript(gradle.substrate.v1.Substrate.EvaluateScriptRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.EvaluateScriptResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getEvaluateScriptMethod(), responseObserver);
    }

    default void getBuildModel(gradle.substrate.v1.Substrate.GetBuildModelRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.GetBuildModelResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getGetBuildModelMethod(), responseObserver);
    }

    default void resolveConfiguration(gradle.substrate.v1.Substrate.ResolveConfigRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ResolveConfigResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getResolveConfigurationMethod(), responseObserver);
    }

    default void getBuildEnvironment(gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getGetBuildEnvironmentMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service JvmHostService.
   */
  public static abstract class JvmHostServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return JvmHostServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service JvmHostService.
   */
  public static final class JvmHostServiceStub
      extends io.grpc.stub.AbstractAsyncStub<JvmHostServiceStub> {
    private JvmHostServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected JvmHostServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new JvmHostServiceStub(channel, callOptions);
    }

    public void evaluateScript(gradle.substrate.v1.Substrate.EvaluateScriptRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.EvaluateScriptResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getEvaluateScriptMethod(), getCallOptions()), request, responseObserver);
    }

    public void getBuildModel(gradle.substrate.v1.Substrate.GetBuildModelRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.GetBuildModelResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getGetBuildModelMethod(), getCallOptions()), request, responseObserver);
    }

    public void resolveConfiguration(gradle.substrate.v1.Substrate.ResolveConfigRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ResolveConfigResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getResolveConfigurationMethod(), getCallOptions()), request, responseObserver);
    }

    public void getBuildEnvironment(gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getGetBuildEnvironmentMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service JvmHostService.
   */
  public static final class JvmHostServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<JvmHostServiceBlockingV2Stub> {
    private JvmHostServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected JvmHostServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new JvmHostServiceBlockingV2Stub(channel, callOptions);
    }

    public gradle.substrate.v1.Substrate.EvaluateScriptResponse evaluateScript(gradle.substrate.v1.Substrate.EvaluateScriptRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getEvaluateScriptMethod(), getCallOptions(), request);
    }

    public gradle.substrate.v1.Substrate.GetBuildModelResponse getBuildModel(gradle.substrate.v1.Substrate.GetBuildModelRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getGetBuildModelMethod(), getCallOptions(), request);
    }

    public gradle.substrate.v1.Substrate.ResolveConfigResponse resolveConfiguration(gradle.substrate.v1.Substrate.ResolveConfigRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getResolveConfigurationMethod(), getCallOptions(), request);
    }

    public gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse getBuildEnvironment(gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getGetBuildEnvironmentMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service JvmHostService.
   */
  public static final class JvmHostServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<JvmHostServiceBlockingStub> {
    private JvmHostServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected JvmHostServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new JvmHostServiceBlockingStub(channel, callOptions);
    }

    public gradle.substrate.v1.Substrate.EvaluateScriptResponse evaluateScript(gradle.substrate.v1.Substrate.EvaluateScriptRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getEvaluateScriptMethod(), getCallOptions(), request);
    }

    public gradle.substrate.v1.Substrate.GetBuildModelResponse getBuildModel(gradle.substrate.v1.Substrate.GetBuildModelRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getGetBuildModelMethod(), getCallOptions(), request);
    }

    public gradle.substrate.v1.Substrate.ResolveConfigResponse resolveConfiguration(gradle.substrate.v1.Substrate.ResolveConfigRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getResolveConfigurationMethod(), getCallOptions(), request);
    }

    public gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse getBuildEnvironment(gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getGetBuildEnvironmentMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service JvmHostService.
   */
  public static final class JvmHostServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<JvmHostServiceFutureStub> {
    private JvmHostServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected JvmHostServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new JvmHostServiceFutureStub(channel, callOptions);
    }

    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.EvaluateScriptResponse> evaluateScript(
        gradle.substrate.v1.Substrate.EvaluateScriptRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getEvaluateScriptMethod(), getCallOptions()), request);
    }

    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.GetBuildModelResponse> getBuildModel(
        gradle.substrate.v1.Substrate.GetBuildModelRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getGetBuildModelMethod(), getCallOptions()), request);
    }

    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.ResolveConfigResponse> resolveConfiguration(
        gradle.substrate.v1.Substrate.ResolveConfigRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getResolveConfigurationMethod(), getCallOptions()), request);
    }

    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse> getBuildEnvironment(
        gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getGetBuildEnvironmentMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_EVALUATE_SCRIPT = 0;
  private static final int METHODID_GET_BUILD_MODEL = 1;
  private static final int METHODID_RESOLVE_CONFIGURATION = 2;
  private static final int METHODID_GET_BUILD_ENVIRONMENT = 3;

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
        case METHODID_EVALUATE_SCRIPT:
          serviceImpl.evaluateScript((gradle.substrate.v1.Substrate.EvaluateScriptRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.EvaluateScriptResponse>) responseObserver);
          break;
        case METHODID_GET_BUILD_MODEL:
          serviceImpl.getBuildModel((gradle.substrate.v1.Substrate.GetBuildModelRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.GetBuildModelResponse>) responseObserver);
          break;
        case METHODID_RESOLVE_CONFIGURATION:
          serviceImpl.resolveConfiguration((gradle.substrate.v1.Substrate.ResolveConfigRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.ResolveConfigResponse>) responseObserver);
          break;
        case METHODID_GET_BUILD_ENVIRONMENT:
          serviceImpl.getBuildEnvironment((gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse>) responseObserver);
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
          getEvaluateScriptMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.EvaluateScriptRequest,
              gradle.substrate.v1.Substrate.EvaluateScriptResponse>(
                service, METHODID_EVALUATE_SCRIPT)))
        .addMethod(
          getGetBuildModelMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.GetBuildModelRequest,
              gradle.substrate.v1.Substrate.GetBuildModelResponse>(
                service, METHODID_GET_BUILD_MODEL)))
        .addMethod(
          getResolveConfigurationMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.ResolveConfigRequest,
              gradle.substrate.v1.Substrate.ResolveConfigResponse>(
                service, METHODID_RESOLVE_CONFIGURATION)))
        .addMethod(
          getGetBuildEnvironmentMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.GetBuildEnvironmentRequest,
              gradle.substrate.v1.Substrate.GetBuildEnvironmentResponse>(
                service, METHODID_GET_BUILD_ENVIRONMENT)))
        .build();
  }

  private static abstract class JvmHostServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    JvmHostServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("JvmHostService");
    }
  }

  private static final class JvmHostServiceFileDescriptorSupplier
      extends JvmHostServiceBaseDescriptorSupplier {
    JvmHostServiceFileDescriptorSupplier() {}
  }

  private static final class JvmHostServiceMethodDescriptorSupplier
      extends JvmHostServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    JvmHostServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (JvmHostServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new JvmHostServiceFileDescriptorSupplier())
              .addMethod(getEvaluateScriptMethod())
              .addMethod(getGetBuildModelMethod())
              .addMethod(getResolveConfigurationMethod())
              .addMethod(getGetBuildEnvironmentMethod())
              .build();
        }
      }
    }
    return result;
  }
}
