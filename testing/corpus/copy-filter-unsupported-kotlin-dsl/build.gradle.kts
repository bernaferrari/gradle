plugins {
    base
}

tasks.register<Copy>("copyFiltered") {
    from("src/raw")
    into(layout.buildDirectory.dir("filtered"))
    outputs.dir("build/filtered")
    filter { line: String -> line.replace("TOKEN", "native-copy-filter") }
}

tasks.named("build") {
    dependsOn(tasks.named("copyFiltered"))
}
