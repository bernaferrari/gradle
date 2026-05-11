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
    implementation("org.gradle.substrate:filtered:1.0")
}

tasks.register("dependencyContractReport") {
    val output = layout.buildDirectory.file("reports/dependency-contract.txt")
    outputs.file("build/reports/dependency-contract.txt")
    doLast {
        output.get().asFile.apply {
            parentFile.mkdirs()
            writeText("repository content filter corpus\n")
        }
    }
}

tasks.named("build") {
    dependsOn(tasks.named("dependencyContractReport"))
}
