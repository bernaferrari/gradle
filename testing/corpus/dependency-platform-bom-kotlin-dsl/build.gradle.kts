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
    implementation(platform("com.squareup.okhttp3:okhttp-bom:4.12.0"))
    implementation("com.squareup.okhttp3:okhttp")
    implementation("com.squareup.okhttp3:logging-interceptor")
}

tasks.register("dependencyContractReport") {
    val output = layout.buildDirectory.file("reports/dependency-contract.txt")
    outputs.file("build/reports/dependency-contract.txt")
    doLast {
        output.get().asFile.apply {
            parentFile.mkdirs()
            writeText("dependency platform bom corpus\n")
        }
    }
}

tasks.named("build") {
    dependsOn(tasks.named("dependencyContractReport"))
}
