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
            url = uri("repo")
            metadataSources {
                gradleMetadata()
                mavenPom()
            }
        }
    }
}

rootProject.name = "corpus-dependency-gradle-metadata-constraints-excludes"
