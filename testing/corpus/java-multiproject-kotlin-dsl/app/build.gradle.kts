plugins {
    application
}

group = "org.gradle.substrate.corpus"
version = "1.0.0"

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

dependencies {
    implementation(project(":lib"))
}

application {
    mainClass.set("org.gradle.substrate.corpus.app.App")
}

tasks.register("contractReport") {
    outputs.dir("build/reports/contract")
}
