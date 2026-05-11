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

dependencies {
    components {
        all {
            status = "release"
        }
    }

    implementation("org.gradle.substrate:metadata-rule:1.0")
}
