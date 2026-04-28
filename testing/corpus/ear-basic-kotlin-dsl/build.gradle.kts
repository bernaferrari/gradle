plugins {
    id("ear")
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

tasks.ear {
    archiveFileName.set("corpus.ear")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    includeEmptyDirs = false
    manifest {
        attributes("Implementation-Title" to "ear-corpus")
    }
}
