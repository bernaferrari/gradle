plugins {
    java
}

group = "example"
version = "1.0.0"

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
}

val resourceTokens = mapOf(
    "appName" to "corpus-java-resources-expand",
    "appVersion" to project.version.toString()
)

tasks.processResources {
    include("**/*.properties")
    inputs.properties(resourceTokens)
    expand(resourceTokens)
}
