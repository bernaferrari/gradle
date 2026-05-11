import org.gradle.api.artifacts.transform.TransformAction
import org.gradle.api.artifacts.transform.TransformOutputs
import org.gradle.api.artifacts.transform.InputArtifact
import org.gradle.api.attributes.Attribute
import org.gradle.api.file.FileSystemLocation
import org.gradle.api.provider.Provider

plugins {
    `java-library`
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

val artifactKind = Attribute.of("org.gradle.substrate.artifact-kind", String::class.java)

abstract class MarkerTransform : TransformAction<org.gradle.api.artifacts.transform.TransformParameters.None> {
    @get:InputArtifact
    abstract val inputArtifact: Provider<FileSystemLocation>

    override fun transform(outputs: TransformOutputs) {
        val input = inputArtifact.get().asFile
        val output = outputs.file(input.nameWithoutExtension + ".marker")
        output.writeText(input.name + "\n")
    }
}

repositories {
    maven {
        url = uri("repo")
    }
}

dependencies {
    attributesSchema {
        attribute(artifactKind)
    }
    artifactTypes.getByName("jar") {
        attributes.attribute(artifactKind, "jar")
    }
    registerTransform(MarkerTransform::class) {
        from.attribute(artifactKind, "jar")
        to.attribute(artifactKind, "marker")
    }
    implementation("org.gradle.substrate:artifact-transform:1.0")
}

configurations.runtimeClasspath {
    attributes.attribute(artifactKind, "marker")
}

tasks.register("resolveTransformedArtifact") {
    outputs.file("build/artifact-transform/resolved.txt")

    doLast {
        val files = configurations.runtimeClasspath.get().files.map { it.name }.sorted()
        file("build/artifact-transform/resolved.txt").apply {
            parentFile.mkdirs()
            writeText(files.joinToString(separator = "\n", postfix = "\n"))
        }
    }
}

tasks.named("build") {
    dependsOn("resolveTransformedArtifact")
}
