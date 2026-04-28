plugins {
    id("base")
}

val packageTar = tasks.register<Tar>("packageTar") {
    from("src/dist")
    archiveFileName.set("corpus.tar.gz")
    compression = Compression.GZIP
    destinationDirectory.set(layout.buildDirectory.dir("distributions"))
    includeEmptyDirs = false
}

tasks.named("assemble") {
    dependsOn(packageTar)
}
