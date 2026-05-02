plugins {
    `java-library`
}

repositories {
    mavenCentral()
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.2")
    testRuntimeOnly("org.junit.platform:junit-platform-console-standalone:1.10.2")
}

tasks.test {
    filter {
        includeTestsMatching("org.gradle.substrate.corpus.junit.*Test")
        excludeTestsMatching("org.gradle.substrate.corpus.junit.Legacy*")
    }
    useJUnitPlatform {
        includeTags("fast")
        excludeTags("slow")
    }
}

tasks.register("contractReport") {
    outputs.dir("build/reports/contract")
}
