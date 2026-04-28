plugins {
    id("base")
}

val syncNested = tasks.register<Sync>("syncNested") {
    into(layout.buildDirectory.dir("synced"))
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
    dependsOn(syncNested)
}
