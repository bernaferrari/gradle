plugins {
    base
}

tasks.register("customJvmTask") {
    val output = layout.buildDirectory.file("custom/task.txt")
    outputs.file("build/custom/task.txt")
    doLast {
        output.get().asFile.apply {
            parentFile.mkdirs()
            writeText("custom JVM task requires compatibility execution\n")
        }
    }
}

tasks.named("build") {
    dependsOn(tasks.named("customJvmTask"))
}
