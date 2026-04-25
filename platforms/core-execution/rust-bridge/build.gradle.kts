plugins {
    id("gradlebuild.distribution.uninstrumented.api-java")
    id("com.google.protobuf")
}

description = "gRPC bridge to the Rust execution substrate daemon"

tasks.register<Sync>("syncProtos") {
    from("${rootProject.projectDir}/substrate/proto/v1")
    into("src/main/proto/v1")
}

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
    implementation(projects.buildCache)
    implementation(projects.coreApi)
    implementation(projects.serviceProvider)
    implementation(projects.buildProcessServices)
    implementation(projects.processServices)
    implementation(projects.processServicesBase)
    implementation(projects.modelCore)
    compileOnly(projects.internalInstrumentationApi)

    testImplementation(projects.testingBase)
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

tasks.named("generateProto").configure { dependsOn("syncProtos") }
tasks.named("processResources").configure { dependsOn("syncProtos") }
tasks.named<ProcessResources>("processResources") {
    // Keep service descriptor packaged so the active lightweight services are discoverable.
}

// Two-phase compilation: compile proto-generated sources first, then handwritten sources.
// This works around javac's inability to resolve proto types when compiling 800+ files
// together with Gradle's strict-compile flags (-XDshouldStopPolicyIfError=FLOW, etc.)

val compileProtoJava by tasks.registering(JavaCompile::class) {
    description = "Compiles protobuf-generated Java and gRPC sources"
    group = "build"

    dependsOn("generateProto")

    val protoJava = layout.buildDirectory.dir("generated/source/proto/main/java")
    val protoGrpc = layout.buildDirectory.dir("generated/source/proto/main/grpc")
    source(protoJava, protoGrpc)

    destinationDirectory.set(layout.buildDirectory.dir("classes/java/proto"))
    classpath = configurations.compileClasspath.get()

    options.encoding = "utf-8"
    options.compilerArgs.addAll(listOf("-proc:none", "-Xlint:-options", "-Werror"))
    // Match the JVM target from the main compilation
    options.release.set(8)
}

tasks.named<JavaCompile>("compileJava") {
    dependsOn(compileProtoJava)

    // Add proto-compiled classes to the classpath
    classpath = files(compileProtoJava.flatMap { it.destinationDirectory }) + classpath

    // Exclude proto-generated sources from the main compile (they're compiled separately).
    val excludedDirs = listOf(
        "build/generated/source/proto"
    )
    setSource(source.filter { file ->
        excludedDirs.none { file.absolutePath.contains(it) }
    })

    // Disable annotation processing for this project
    options.compilerArgs.add("-proc:none")
}

// Exclude test files that still depend on excluded main sources
tasks.named<JavaCompile>("compileTestJava") {
    exclude("**/jvmhost/JvmHostServerTest.java")
}

// Add proto-compiled classes to the main source set output so tests can see them
sourceSets.main.get().output.dir(
    mapOf("builtBy" to compileProtoJava),
    compileProtoJava.flatMap { it.destinationDirectory }
)

gradleModule {
    requiredRuntimes {
        daemon = true
    }
    computedRuntimes {
        daemon = true
        client = true
    }
}
