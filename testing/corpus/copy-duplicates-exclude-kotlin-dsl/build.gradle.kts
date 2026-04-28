plugins {
    id("base")
}

val copyMerged = tasks.register<Copy>("copyMerged") {
    from("src/a")
    from("src/b")
    into(layout.buildDirectory.dir("merged"))
    include("**/*.txt")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    outputs.dir("build/merged")
}

tasks.named("build") {
    dependsOn(copyMerged)
}
