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
    implementation("org.gradle.substrate:artifact-view:1.0")
}

tasks.register("resolveArtifactView") {
    outputs.file("build/artifact-view/resolved.txt")

    doLast {
        val view = configurations.runtimeClasspath.get().incoming.artifactView {
            lenient(true)
        }
        val files = view.files.files.map { it.name }.sorted()
        file("build/artifact-view/resolved.txt").writeText(files.joinToString(separator = "\n", postfix = "\n"))
    }
}

tasks.named("build") {
    dependsOn("resolveArtifactView")
}
