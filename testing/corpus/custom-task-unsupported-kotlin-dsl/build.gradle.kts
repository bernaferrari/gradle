plugins {
    base
}

tasks.register("customJvmTask") {
    val output = layout.buildDirectory.file("custom/task.txt")
    val message = "custom JVM task requires compatibility execution\n"
    outputs.file("build/custom/task.txt")
    doLast {
        output.get().asFile.apply {
            parentFile.mkdirs()
            writeText(message)
        }
    }
}

tasks.named("build") {
    dependsOn(tasks.named("customJvmTask"))
}
