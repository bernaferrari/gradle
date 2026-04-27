plugins {
    id("base")
}

val filterResources = tasks.register<Copy>("filterResources") {
    from("src/raw")
    into(layout.buildDirectory.dir("resources/main"))
    include("**/*.properties")
    exclude("**/secret.*")
    outputs.dir("build/resources/main")
}

tasks.named("build") {
    dependsOn(filterResources)
}
