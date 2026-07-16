plugins {
    kotlin("jvm") version "2.4.0"
    `java-library`
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
}

kotlin {
    jvmToolchain(17)
}

tasks.test {
    useJUnitPlatform()
}

dependencies {
    testImplementation(kotlin("test"))
}
