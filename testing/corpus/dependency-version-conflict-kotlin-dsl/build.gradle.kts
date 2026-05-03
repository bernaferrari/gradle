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
    implementation("org.slf4j:slf4j-api:2.0.13")
    implementation("ch.qos.logback:logback-classic:1.5.6")
}

tasks.register("dependencyContractReport") {
    val output = layout.buildDirectory.file("reports/dependency-contract.txt")
    outputs.file("build/reports/dependency-contract.txt")
    doLast {
        output.get().asFile.apply {
            parentFile.mkdirs()
            writeText("dependency version conflict corpus\n")
        }
    }
}

tasks.named("build") {
    dependsOn(tasks.named("dependencyContractReport"))
}
