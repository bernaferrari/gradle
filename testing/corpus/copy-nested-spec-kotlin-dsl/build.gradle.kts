plugins {
    id("base")
}

val stageNested = tasks.register<Copy>("stageNested") {
    into(layout.buildDirectory.dir("staged"))
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
    dependsOn(stageNested)
}
