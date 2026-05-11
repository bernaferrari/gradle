plugins {
    `java-library`
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

dependencies {
    implementation("org.gradle.substrate:included-lib:1.0")
}
