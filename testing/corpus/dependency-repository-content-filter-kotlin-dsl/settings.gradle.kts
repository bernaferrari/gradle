pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        maven {
            name = "rejected"
            url = uri("repo-rejected")
            content {
                includeGroup("org.other")
            }
        }
        maven {
            name = "selected"
            url = uri("repo-selected")
            content {
                includeGroup("org.gradle.substrate")
                includeModule("org.gradle.substrate", "filtered")
                includeVersion("org.gradle.substrate", "filtered", "1.0")
            }
        }
    }
}

rootProject.name = "corpus-dependency-repository-content-filter"
