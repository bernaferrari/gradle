pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

rootProject.name = "composite-substitution-unsupported"

includeBuild("included-lib")
