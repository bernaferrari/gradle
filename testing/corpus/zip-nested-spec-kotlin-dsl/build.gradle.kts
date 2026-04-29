plugins {
    id("base")
}

val packageZip = tasks.register<Zip>("packageZip") {
    archiveFileName.set("nested.zip")
    destinationDirectory.set(layout.buildDirectory.dir("libs"))
    from("src/root")
    into("config") {
        from("src/config")
    }
    into("public/assets") {
        from("src/assets")
    }
    includeEmptyDirs = false
    duplicatesStrategy = DuplicatesStrategy.FAIL
}

tasks.named("assemble") {
    dependsOn(packageZip)
}
