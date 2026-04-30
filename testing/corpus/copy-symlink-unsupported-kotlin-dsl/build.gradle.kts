plugins {
    base
}

tasks.register<Copy>("copySymlinkInput") {
    from("src/source")
    into(layout.buildDirectory.dir("copied"))
    outputs.dir("build/copied")
}

tasks.named("build") {
    dependsOn(tasks.named("copySymlinkInput"))
}
