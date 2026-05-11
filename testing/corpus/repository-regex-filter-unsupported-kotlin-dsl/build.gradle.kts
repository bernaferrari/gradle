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
        content {
            includeGroupByRegex("org\\.gradle\\.substrate")
        }
    }
}

dependencies {
    implementation("org.gradle.substrate:regex-filtered:1.0")
}
