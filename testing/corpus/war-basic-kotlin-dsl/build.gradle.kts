plugins {
    id("war")
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
}

tasks.war {
    archiveFileName.set("corpus.war")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    includeEmptyDirs = false
    manifest {
        attributes(
            "Implementation-Title" to "war-corpus",
            "Main-Class" to "example.WarApp",
        )
    }
}
