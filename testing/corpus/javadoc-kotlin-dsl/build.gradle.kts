import org.gradle.external.javadoc.StandardJavadocDocletOptions

plugins {
    java
}

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
}

tasks.withType<Javadoc>().configureEach {
    title = "Native Javadoc Corpus"
    options.encoding = "UTF-8"
    (options as StandardJavadocDocletOptions).noTimestamp()
}

tasks.named("build") {
    dependsOn(tasks.named("javadoc"))
}
