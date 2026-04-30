import org.gradle.external.javadoc.StandardJavadocDocletOptions

plugins {
    `java-library`
}

group = "example"
version = "1.0.0"

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
    withSourcesJar()
}

tasks.withType<Javadoc>().configureEach {
    title = "OSS Style API"
    options.encoding = "UTF-8"
    (options as StandardJavadocDocletOptions).noTimestamp()
}

tasks.register("apiContractReport") {
    outputs.file("build/reports/api-contract.txt")
    doLast {
        val output = layout.buildDirectory.file("reports/api-contract.txt").get().asFile
        output.parentFile.mkdirs()
        output.writeText("oss-style api contract\n")
    }
}

tasks.named("build") {
    dependsOn(tasks.named("javadoc"))
    dependsOn(tasks.named("sourcesJar"))
    dependsOn(tasks.named("apiContractReport"))
}
