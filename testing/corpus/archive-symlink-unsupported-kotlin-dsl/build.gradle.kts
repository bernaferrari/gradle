plugins {
    base
}

tasks.register<Zip>("zipSymlinkInput") {
    from("src/source")
    archiveFileName.set("symlink-input.zip")
    destinationDirectory.set(layout.buildDirectory.dir("distributions"))
    outputs.file("build/distributions/symlink-input.zip")
}

tasks.named("build") {
    dependsOn(tasks.named("zipSymlinkInput"))
}
