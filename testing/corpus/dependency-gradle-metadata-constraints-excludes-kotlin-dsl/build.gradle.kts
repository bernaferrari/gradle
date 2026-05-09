plugins {
    base
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

val runtimeClasspath = configurations.create("runtimeClasspath") {
    isCanBeResolved = true
    isCanBeConsumed = false
}

dependencies {
    runtimeClasspath("org.example:root:1.0")
}
