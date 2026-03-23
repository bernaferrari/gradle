plugins {
    id("gradlebuild.distribution.api-java")
    id("com.google.protobuf")
}

description = "gRPC bridge to the Rust execution substrate daemon"

dependencies {
    api(projects.baseServices)
    api(projects.hashing)
    api(libs.jspecify)

    implementation(libs.guava)
    implementation(libs.inject)
    implementation(libs.grpc)
    implementation(libs.grpcStub)
    implementation(libs.grpcProtobuf)
    implementation(libs.grpcNettyShaded)
    implementation(projects.execution)
    implementation(projects.snapshots)
    implementation(projects.fileWatching)
    implementation(projects.buildOperations)
    implementation(projects.testingBase)

    testImplementation(projects.baseServicesGroovy)
    testImplementation(testFixtures(projects.baseServices))
    testImplementation(testFixtures(projects.execution))
}

protobuf {
    protoc {
        artifact = "com.google.protobuf:protoc:3.25.3"
    }
    plugins {
        create("grpc") {
            artifact = "io.grpc:protoc-gen-grpc-java:1.62.2"
        }
    }
    generateProtoTasks {
        all().forEach { task ->
            task.plugins {
                create("grpc")
            }
        }
    }
}

gradleModule {
    requiredRuntimes {
        daemon = true
    }
    computedRuntimes {
        daemon = true
    }
}
