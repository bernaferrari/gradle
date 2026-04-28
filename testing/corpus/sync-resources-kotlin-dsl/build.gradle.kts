plugins {
    id("base")
}

val resourceTokens = mapOf("appName" to "sync-corpus")

val syncResources = tasks.register<Sync>("syncResources") {
    from("src/resources-a")
    from("src/resources-b")
    into(layout.buildDirectory.dir("synced"))
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE

    inputs.properties(resourceTokens)
    outputs.dir("build/synced")
    include("**/*.properties")
    expand(resourceTokens)
}

tasks.named("build") {
    dependsOn(syncResources)
}
