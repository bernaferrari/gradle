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
    implementation("org.apache.commons:commons-compress:1.26.1")
    implementation("com.google.code.gson:gson:2.11.0")
}

tasks.register("dependencyContractReport") {
    val output = layout.buildDirectory.file("reports/dependency-contract.txt")
    outputs.file("build/reports/dependency-contract.txt")
    doLast {
        output.get().asFile.apply {
            parentFile.mkdirs()
            writeText("dependency transitive chain corpus\n")
        }
    }
}

tasks.named("build") {
    dependsOn(tasks.named("dependencyContractReport"))
}
