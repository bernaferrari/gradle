plugins {
    java
}

val javaLauncher = javaToolchains.launcherFor {
    languageVersion.set(JavaLanguageVersion.of(17))
}

val probeJava = tasks.register<Exec>("probeJava") {
    executable = javaLauncher.get().executablePath.asFile.absolutePath
    args("-version")
}

tasks.named("build") {
    dependsOn(probeJava)
}
