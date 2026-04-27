plugins {
    `java-library`
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

tasks.register("generateSources") {
    outputs.dir("build/generated/contract")
}

tasks.register("contractReport") {
    dependsOn("generateSources")
    outputs.dir("build/reports/contract")
}
