package gradle.substrate.v1;

import static io.grpc.MethodDescriptor.generateFullMethodName;

/**
 * <pre>
 * Walks file trees and computes content hashes in Rust.
 * Replaces Java's FileCollectionFingerprinter.
 * </pre>
 */
@io.grpc.stub.annotations.GrpcGenerated
public final class FileFingerprintServiceGrpc {

  private FileFingerprintServiceGrpc() {}

  public static final java.lang.String SERVICE_NAME = "gradle.substrate.v1.FileFingerprintService";

  // Static method descriptors that strictly reflect the proto.
  private static volatile io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.FingerprintFilesRequest,
      gradle.substrate.v1.Substrate.FingerprintFilesResponse> getFingerprintFilesMethod;

  @io.grpc.stub.annotations.RpcMethod(
      fullMethodName = SERVICE_NAME + '/' + "FingerprintFiles",
      requestType = gradle.substrate.v1.Substrate.FingerprintFilesRequest.class,
      responseType = gradle.substrate.v1.Substrate.FingerprintFilesResponse.class,
      methodType = io.grpc.MethodDescriptor.MethodType.UNARY)
  public static io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.FingerprintFilesRequest,
      gradle.substrate.v1.Substrate.FingerprintFilesResponse> getFingerprintFilesMethod() {
    io.grpc.MethodDescriptor<gradle.substrate.v1.Substrate.FingerprintFilesRequest, gradle.substrate.v1.Substrate.FingerprintFilesResponse> getFingerprintFilesMethod;
    if ((getFingerprintFilesMethod = FileFingerprintServiceGrpc.getFingerprintFilesMethod) == null) {
      synchronized (FileFingerprintServiceGrpc.class) {
        if ((getFingerprintFilesMethod = FileFingerprintServiceGrpc.getFingerprintFilesMethod) == null) {
          FileFingerprintServiceGrpc.getFingerprintFilesMethod = getFingerprintFilesMethod =
              io.grpc.MethodDescriptor.<gradle.substrate.v1.Substrate.FingerprintFilesRequest, gradle.substrate.v1.Substrate.FingerprintFilesResponse>newBuilder()
              .setType(io.grpc.MethodDescriptor.MethodType.UNARY)
              .setFullMethodName(generateFullMethodName(SERVICE_NAME, "FingerprintFiles"))
              .setSampledToLocalTracing(true)
              .setRequestMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.FingerprintFilesRequest.getDefaultInstance()))
              .setResponseMarshaller(io.grpc.protobuf.ProtoUtils.marshaller(
                  gradle.substrate.v1.Substrate.FingerprintFilesResponse.getDefaultInstance()))
              .setSchemaDescriptor(new FileFingerprintServiceMethodDescriptorSupplier("FingerprintFiles"))
              .build();
        }
      }
    }
    return getFingerprintFilesMethod;
  }

  /**
   * Creates a new async stub that supports all call types for the service
   */
  public static FileFingerprintServiceStub newStub(io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceStub>() {
        @java.lang.Override
        public FileFingerprintServiceStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new FileFingerprintServiceStub(channel, callOptions);
        }
      };
    return FileFingerprintServiceStub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports all types of calls on the service
   */
  public static FileFingerprintServiceBlockingV2Stub newBlockingV2Stub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceBlockingV2Stub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceBlockingV2Stub>() {
        @java.lang.Override
        public FileFingerprintServiceBlockingV2Stub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new FileFingerprintServiceBlockingV2Stub(channel, callOptions);
        }
      };
    return FileFingerprintServiceBlockingV2Stub.newStub(factory, channel);
  }

  /**
   * Creates a new blocking-style stub that supports unary and streaming output calls on the service
   */
  public static FileFingerprintServiceBlockingStub newBlockingStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceBlockingStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceBlockingStub>() {
        @java.lang.Override
        public FileFingerprintServiceBlockingStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new FileFingerprintServiceBlockingStub(channel, callOptions);
        }
      };
    return FileFingerprintServiceBlockingStub.newStub(factory, channel);
  }

  /**
   * Creates a new ListenableFuture-style stub that supports unary calls on the service
   */
  public static FileFingerprintServiceFutureStub newFutureStub(
      io.grpc.Channel channel) {
    io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceFutureStub> factory =
      new io.grpc.stub.AbstractStub.StubFactory<FileFingerprintServiceFutureStub>() {
        @java.lang.Override
        public FileFingerprintServiceFutureStub newStub(io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
          return new FileFingerprintServiceFutureStub(channel, callOptions);
        }
      };
    return FileFingerprintServiceFutureStub.newStub(factory, channel);
  }

  /**
   * <pre>
   * Walks file trees and computes content hashes in Rust.
   * Replaces Java's FileCollectionFingerprinter.
   * </pre>
   */
  public interface AsyncService {

    /**
     * <pre>
     * Fingerprint a set of files/directories.
     * </pre>
     */
    default void fingerprintFiles(gradle.substrate.v1.Substrate.FingerprintFilesRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.FingerprintFilesResponse> responseObserver) {
      io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall(getFingerprintFilesMethod(), responseObserver);
    }
  }

  /**
   * Base class for the server implementation of the service FileFingerprintService.
   * <pre>
   * Walks file trees and computes content hashes in Rust.
   * Replaces Java's FileCollectionFingerprinter.
   * </pre>
   */
  public static abstract class FileFingerprintServiceImplBase
      implements io.grpc.BindableService, AsyncService {

    @java.lang.Override public final io.grpc.ServerServiceDefinition bindService() {
      return FileFingerprintServiceGrpc.bindService(this);
    }
  }

  /**
   * A stub to allow clients to do asynchronous rpc calls to service FileFingerprintService.
   * <pre>
   * Walks file trees and computes content hashes in Rust.
   * Replaces Java's FileCollectionFingerprinter.
   * </pre>
   */
  public static final class FileFingerprintServiceStub
      extends io.grpc.stub.AbstractAsyncStub<FileFingerprintServiceStub> {
    private FileFingerprintServiceStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected FileFingerprintServiceStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new FileFingerprintServiceStub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a set of files/directories.
     * </pre>
     */
    public void fingerprintFiles(gradle.substrate.v1.Substrate.FingerprintFilesRequest request,
        io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.FingerprintFilesResponse> responseObserver) {
      io.grpc.stub.ClientCalls.asyncUnaryCall(
          getChannel().newCall(getFingerprintFilesMethod(), getCallOptions()), request, responseObserver);
    }
  }

  /**
   * A stub to allow clients to do synchronous rpc calls to service FileFingerprintService.
   * <pre>
   * Walks file trees and computes content hashes in Rust.
   * Replaces Java's FileCollectionFingerprinter.
   * </pre>
   */
  public static final class FileFingerprintServiceBlockingV2Stub
      extends io.grpc.stub.AbstractBlockingStub<FileFingerprintServiceBlockingV2Stub> {
    private FileFingerprintServiceBlockingV2Stub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected FileFingerprintServiceBlockingV2Stub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new FileFingerprintServiceBlockingV2Stub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a set of files/directories.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.FingerprintFilesResponse fingerprintFiles(gradle.substrate.v1.Substrate.FingerprintFilesRequest request) throws io.grpc.StatusException {
      return io.grpc.stub.ClientCalls.blockingV2UnaryCall(
          getChannel(), getFingerprintFilesMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do limited synchronous rpc calls to service FileFingerprintService.
   * <pre>
   * Walks file trees and computes content hashes in Rust.
   * Replaces Java's FileCollectionFingerprinter.
   * </pre>
   */
  public static final class FileFingerprintServiceBlockingStub
      extends io.grpc.stub.AbstractBlockingStub<FileFingerprintServiceBlockingStub> {
    private FileFingerprintServiceBlockingStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected FileFingerprintServiceBlockingStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new FileFingerprintServiceBlockingStub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a set of files/directories.
     * </pre>
     */
    public gradle.substrate.v1.Substrate.FingerprintFilesResponse fingerprintFiles(gradle.substrate.v1.Substrate.FingerprintFilesRequest request) {
      return io.grpc.stub.ClientCalls.blockingUnaryCall(
          getChannel(), getFingerprintFilesMethod(), getCallOptions(), request);
    }
  }

  /**
   * A stub to allow clients to do ListenableFuture-style rpc calls to service FileFingerprintService.
   * <pre>
   * Walks file trees and computes content hashes in Rust.
   * Replaces Java's FileCollectionFingerprinter.
   * </pre>
   */
  public static final class FileFingerprintServiceFutureStub
      extends io.grpc.stub.AbstractFutureStub<FileFingerprintServiceFutureStub> {
    private FileFingerprintServiceFutureStub(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      super(channel, callOptions);
    }

    @java.lang.Override
    protected FileFingerprintServiceFutureStub build(
        io.grpc.Channel channel, io.grpc.CallOptions callOptions) {
      return new FileFingerprintServiceFutureStub(channel, callOptions);
    }

    /**
     * <pre>
     * Fingerprint a set of files/directories.
     * </pre>
     */
    public com.google.common.util.concurrent.ListenableFuture<gradle.substrate.v1.Substrate.FingerprintFilesResponse> fingerprintFiles(
        gradle.substrate.v1.Substrate.FingerprintFilesRequest request) {
      return io.grpc.stub.ClientCalls.futureUnaryCall(
          getChannel().newCall(getFingerprintFilesMethod(), getCallOptions()), request);
    }
  }

  private static final int METHODID_FINGERPRINT_FILES = 0;

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
        case METHODID_FINGERPRINT_FILES:
          serviceImpl.fingerprintFiles((gradle.substrate.v1.Substrate.FingerprintFilesRequest) request,
              (io.grpc.stub.StreamObserver<gradle.substrate.v1.Substrate.FingerprintFilesResponse>) responseObserver);
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
          getFingerprintFilesMethod(),
          io.grpc.stub.ServerCalls.asyncUnaryCall(
            new MethodHandlers<
              gradle.substrate.v1.Substrate.FingerprintFilesRequest,
              gradle.substrate.v1.Substrate.FingerprintFilesResponse>(
                service, METHODID_FINGERPRINT_FILES)))
        .build();
  }

  private static abstract class FileFingerprintServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoFileDescriptorSupplier, io.grpc.protobuf.ProtoServiceDescriptorSupplier {
    FileFingerprintServiceBaseDescriptorSupplier() {}

    @java.lang.Override
    public com.google.protobuf.Descriptors.FileDescriptor getFileDescriptor() {
      return gradle.substrate.v1.Substrate.getDescriptor();
    }

    @java.lang.Override
    public com.google.protobuf.Descriptors.ServiceDescriptor getServiceDescriptor() {
      return getFileDescriptor().findServiceByName("FileFingerprintService");
    }
  }

  private static final class FileFingerprintServiceFileDescriptorSupplier
      extends FileFingerprintServiceBaseDescriptorSupplier {
    FileFingerprintServiceFileDescriptorSupplier() {}
  }

  private static final class FileFingerprintServiceMethodDescriptorSupplier
      extends FileFingerprintServiceBaseDescriptorSupplier
      implements io.grpc.protobuf.ProtoMethodDescriptorSupplier {
    private final java.lang.String methodName;

    FileFingerprintServiceMethodDescriptorSupplier(java.lang.String methodName) {
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
      synchronized (FileFingerprintServiceGrpc.class) {
        result = serviceDescriptor;
        if (result == null) {
          serviceDescriptor = result = io.grpc.ServiceDescriptor.newBuilder(SERVICE_NAME)
              .setSchemaDescriptor(new FileFingerprintServiceFileDescriptorSupplier())
              .addMethod(getFingerprintFilesMethod())
              .build();
        }
      }
    }
    return result;
  }
}
