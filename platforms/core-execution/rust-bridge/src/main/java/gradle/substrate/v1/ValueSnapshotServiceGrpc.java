package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 * <pre>
 * Computes fingerprints for input properties in Rust.
 * Replaces Java's DefaultValueSnapshotter.
 * </pre>
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class ValueSnapshotServiceGrpc {

  private ValueSnapshotServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.ValueSnapshotService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.SnapshotValuesRequest,
      gradle.substrate.v1.Substrate.SnapshotValuesResponse> getSnapshotValuesMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "SnapshotValues",
      requestType = gradle.substrate.v1.Substrate.SnapshotValuesRequest.class,
      responseType = gradle.substrate.v1.Substrate.SnapshotValuesResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.SnapshotValuesRequest,
      gradle.substrate.v1.Substrate.SnapshotValuesResponse> getSnapshotValuesMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.SnapshotValuesRequest, gradle.substrate.v1.Substrate.SnapshotValuesResponse> getSnapshotValuesMethod;
    if ((getSnapshotValuesMethod = ValueSnapshotServiceGrpc.getSnapshotValuesMethod) == null) {
      synchronized (ValueSnapshotServiceGrpc.class) {
        if ((getSnapshotValuesMethod = ValueSnapshotServiceGrpc.getSnapshotValuesMethod) == null) {
          ValueSnapshotServiceGrpc.getSnapshotValuesMethod = getSnapshotValuesMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.SnapshotValuesRequest, gradle.substrate.v1.Substrate.SnapshotValuesResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "SnapshotValues"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.SnapshotValuesRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.SnapshotValuesResponse.getDefaultInstance()))
              .setSchemaDescriptor(new ValueSnapshotServiceMethodDescriptorSupplier("SnapshotValues"))
              .build();
        }
      }
    }
    return getSnapshotValuesMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static ValueSnapshotServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceStub>() {
        @java.lang.Override
        public ValueSnapshotServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ValueSnapshotServiceStub(channel, callOptions);
        }
      };
    return ValueSnapshotServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static ValueSnapshotServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceBlockingV2Stub>() {
        @java.lang.Override
        public ValueSnapshotServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ValueSnapshotServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return ValueSnapshotServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static ValueSnapshotServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceBlockingStub>() {
        @java.lang.Override
        public ValueSnapshotServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ValueSnapshotServiceBlockingStub(channel, callOptions);
        }
      };
    return ValueSnapshotServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static ValueSnapshotServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<ValueSnapshotServiceFutureStub>() {
        @java.lang.Override
        public ValueSnapshotServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new ValueSnapshotServiceFutureStub(channel, callOptions);
        }
      };
    return ValueSnapshotServiceFutureStub.newStub(factory, channel);
  }

  /**
   * <pre>
   * Computes fingerprints for input properties in Rust.
   * Replaces Java's DefaultValueSnapshotter.
   * </pre>
   */
  public interface AsyncService {

    /**
     * <pre>
     * Fingerprint a batch of input properties.
     * </pre>
     */
    default void snapshotValues(gradle.substrate.v1.Substrate.SnapshotValuesRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.SnapshotValuesResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getSnapshotValuesMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service ValueSnapshotService.
   * <pre>
   * Computes fingerprints for input properties in Rust.
   * Replaces Java's DefaultValueSnapshotter.
   * </pre>
   */
  public static abstract class ValueSnapshotServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return ValueSnapshotServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service ValueSnapshotService.
   * <pre>
   * Computes fingerprints for input properties in Rust.
   * Replaces Java's DefaultValueSnapshotter.
   * </pre>
   */
  public static final class ValueSnapshotServiceStub
      extends io.grpc.stub.AbstractAsyncStub<ValueSnapshotServiceStub> {
    private ValueSnapshotServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ValueSnapshotServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ValueSnapshotServiceStub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a batch of input properties.
     * </pre>
     */
    public void snapshotValues(gradle.substrate.v1.Substrate.SnapshotValuesRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.SnapshotValuesResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getSnapshotValuesMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service ValueSnapshotService.
   * <pre>
   * Computes fingerprints for input properties in Rust.
   * Replaces Java's DefaultValueSnapshotter.
   * </pre>
   */
  public static final class ValueSnapshotServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<ValueSnapshotServiceBlockingV2Stub> {
    private ValueSnapshotServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ValueSnapshotServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ValueSnapshotServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a batch of input properties.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.SnapshotValuesResponse snapshotValues(gradle.substrate.v1.Substrate.SnapshotValuesRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getSnapshotValuesMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service ValueSnapshotService.
   * <pre>
   * Computes fingerprints for input properties in Rust.
   * Replaces Java's DefaultValueSnapshotter.
   * </pre>
   */
  public static final class ValueSnapshotServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<ValueSnapshotServiceBlockingStub> {
    private ValueSnapshotServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ValueSnapshotServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ValueSnapshotServiceBlockingStub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a batch of input properties.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.SnapshotValuesResponse snapshotValues(gradle.substrate.v1.Substrate.SnapshotValuesRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getSnapshotValuesMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service ValueSnapshotService.
   * <pre>
   * Computes fingerprints for input properties in Rust.
   * Replaces Java's DefaultValueSnapshotter.
   * </pre>
   */
  public static final class ValueSnapshotServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<ValueSnapshotServiceFutureStub> {
    private ValueSnapshotServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected ValueSnapshotServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new ValueSnapshotServiceFutureStub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a batch of input properties.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.SnapshotValuesResponse> snapshotValues(
        gradle.substrate.v1.Substrate.SnapshotValuesRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getSnapshotValuesMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_SNAPSHOT_VALUES = 0;

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
        case METHODID_SNAPSHOT_VALUES:
          serviceImpl.snapshotValues((gradle.substrate.v1.Substrate.SnapshotValuesRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.SnapshotValuesResponse>) responseObserver);
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
          getSnapshotValuesMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.SnapshotValuesRequest,
              gradle.substrate.v1.Substrate.SnapshotValuesResponse>(
                service, METHODID_SNAPSHOT_VALUES)))
        .build();
  }

  private static abstract class ValueSnapshotServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    ValueSnapshotServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("ValueSnapshotService");
    }
  }

  private static final class ValueSnapshotServiceFileDescriptorSupplier
      extends ValueSnapshotServiceBaseDescriptorSupplier {
    ValueSnapshotServiceFileDescriptorSupplier() {}
  }

  private static final class ValueSnapshotServiceMethodDescriptorSupplier
      extends ValueSnapshotServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    ValueSnapshotServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (ValueSnapshotServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new ValueSnapshotServiceFileDescriptorSupplier())
              .addMethod(getSnapshotValuesMethod())
              .build();
        }
      }
    }
    return result;
  }
}
