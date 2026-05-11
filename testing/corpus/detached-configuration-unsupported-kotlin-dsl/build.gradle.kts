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

val detachedDependency = dependencies.create("org.gradle.substrate:detached:1.0")

tasks.register("resolveDetached") {
    val detached = configurations.detachedConfiguration(detachedDependency)

    outputs.file("build/detached/resolved.txt")

    doLast {
        val files = detached.resolve().map { it.name }.sorted()
        file("build/detached/resolved.txt").writeText(files.joinToString(separator = "\n", postfix = "\n"))
    }
}

tasks.named("build") {
    dependsOn("resolveDetached")
}
