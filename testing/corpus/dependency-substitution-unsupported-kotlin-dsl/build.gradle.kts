plugins {
    `java-library`
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

repositories {
    maven {
        url = uri("repo")
    }
}

configurations.configureEach {
    resolutionStrategy.dependencySubstitution {
        substitute(module("org.gradle.substrate:original")).using(module("org.gradle.substrate:replacement:1.0"))
    }
}

dependencies {
    implementation("org.gradle.substrate:original:1.0")
}
