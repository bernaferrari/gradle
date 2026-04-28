plugins {
    id("base")
}

val packageZip = tasks.register<Zip>("packageZip") {
    from("src/dist")
    archiveFileName.set("corpus.zip")
    destinationDirectory.set(layout.buildDirectory.dir("libs"))
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    includeEmptyDirs = false
}

tasks.named("assemble") {
    dependsOn(packageZip)
}
