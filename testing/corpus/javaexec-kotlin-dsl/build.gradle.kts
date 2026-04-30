plugins {
    java
}

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
}

val launcherProvider = javaToolchains.launcherFor {
    languageVersion.set(JavaLanguageVersion.of(17))
}

val runTool = tasks.register<JavaExec>("runTool") {
    dependsOn(tasks.named("classes"))
    javaLauncher.set(launcherProvider)
    classpath = sourceSets.main.get().runtimeClasspath
    mainClass.set("example.Tool")

    val outputFile = layout.buildDirectory.file("resources/javaexec-result.txt")
    args(outputFile.get().asFile.absolutePath, "expected-token")
    outputs.file(outputFile)
}

tasks.named("build") {
    dependsOn(runTool)
}
