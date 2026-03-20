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

    testImplementation(testFixtures(projects.baseServices))
}

gradleModule {
    requiredRuntimes {
        daemon = true
    }
    computedRuntimes {
        daemon = true
    }
}
