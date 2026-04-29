plugins {
    id("base")
}

val packageTar = tasks.register<Tar>("packageTar") {
    archiveFileName.set("nested.tar")
    destinationDirectory.set(layout.buildDirectory.dir("distributions"))
    compression = Compression.NONE
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
    dependsOn(packageTar)
}
