pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

rootProject.name = "composite-substitution-unsupported"

// Structured composite IR captures each includeBuild path and keeps execution fail-closed.
includeBuild("included-lib")
