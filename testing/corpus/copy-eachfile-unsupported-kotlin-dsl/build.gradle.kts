plugins {
    base
}

tasks.register<Copy>("copyWithAction") {
    from("src/raw")
    into(layout.buildDirectory.dir("rewritten"))
    outputs.dir("build/rewritten")
    eachFile {
        relativePath = RelativePath(true, "renamed", name)
    }
}

tasks.named("build") {
    dependsOn(tasks.named("copyWithAction"))
}
