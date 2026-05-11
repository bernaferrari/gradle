plugins {
    `java-library`
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

repositories {
    maven {
        url = uri("repo")
    }
}

dependencies {
    implementation(enforcedPlatform("org.gradle.substrate:enforced-bom:1.0"))
    implementation("org.gradle.substrate:enforced-lib")
}
