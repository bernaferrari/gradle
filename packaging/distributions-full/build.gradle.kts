import gradlebuild.basics.BuildEnvironmentExtension

plugins {
    id("gradlebuild.distribution.packaging")
    id("gradlebuild.verify-build-environment")
    id("gradlebuild.install")
}

description = "The collector project for the entirety of the Gradle distribution"

dependencies {
    coreRuntimeOnly(platform(projects.corePlatform))

    agentsRuntimeOnly(projects.instrumentationAgent)

    pluginsRuntimeOnly(platform(projects.distributionsPublishing))
    pluginsRuntimeOnly(platform(projects.distributionsJvm))
    pluginsRuntimeOnly(platform(projects.distributionsNative))

    pluginsRuntimeOnly(projects.pluginDevelopment)
    pluginsRuntimeOnly(projects.buildConfiguration)
    pluginsRuntimeOnly(projects.buildInit)
    pluginsRuntimeOnly(projects.wrapperMain) {
        because("Need to include the wrapper source in the distribution")
    }
    pluginsRuntimeOnly(projects.buildProfile)
    pluginsRuntimeOnly(projects.antlr)
    pluginsRuntimeOnly(projects.enterprise)
    pluginsRuntimeOnly(projects.unitTestFixtures)
}

// This is required for the separate promotion build and should be adjusted there in the future
val buildEnvironmentExtension = extensions.getByType(BuildEnvironmentExtension::class)
tasks.register<Copy>("copyDistributionsToRootBuild") {
    dependsOn("buildDists")
    from(layout.buildDirectory.dir("distributions"))
    into(buildEnvironmentExtension.rootProjectBuildDir.dir("distributions"))
}

// Copy the substrate daemon binary into the distribution if it exists
tasks.register<Copy>("copySubstrateBinary") {
    description = "Copies the substrate daemon binary into the distribution lib/ directory"
    dependsOn("buildDists")
    val substrateRoot = rootProject.layout.projectDirectory.dir("substrate")
    val osName = System.getProperty("os.name").lowercase()
    val osArch = System.getProperty("os.arch").lowercase()
    val target = when {
        osName.contains("mac") && osArch.contains("aarch64") -> "aarch64-apple-darwin"
        osName.contains("mac") -> "x86_64-apple-darwin"
        osName.contains("win") -> "x86_64-pc-windows-msvc"
        else -> "x86_64-unknown-linux-gnu"
    }
    val binaryDir = substrateRoot.dir("target/$target/release")
    val binaryName = if (osName.contains("win")) "gradle-substrate-daemon.exe" else "gradle-substrate-daemon"
    from(binaryDir) {
        include(binaryName)
    }
    into(layout.buildDirectory.dir("distributions/gradle/lib"))
    onlyIf {
        binaryDir.dir(binaryName).asFile.exists() || binaryDir.file(binaryName).asFile.exists()
    }
}
