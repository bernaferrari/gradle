plugins {
    id("gradlebuild.distribution.api-java")
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

    testImplementation(projects.baseServicesGroovy)
    testImplementation(testFixtures(projects.baseServices))
    testImplementation(testFixtures(projects.execution))
}

gradleModule {
    requiredRuntimes {
        daemon = true
    }
    computedRuntimes {
        daemon = true
    }
}
