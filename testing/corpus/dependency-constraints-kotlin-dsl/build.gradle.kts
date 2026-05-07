plugins {
    `java-library`
}

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
}

dependencies {
    constraints {
        implementation("org.apache.commons:commons-lang3:3.14.0")
    }
    implementation("org.apache.commons:commons-lang3:3.12.0")
    implementation("com.google.guava:guava:33.2.1-jre") {
        exclude(group = "com.google.code.findbugs", module = "jsr305")
    }
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.2")
}

tasks.register("dependencyContractReport") {
    val output = layout.buildDirectory.file("reports/dependency-contract.txt")
    outputs.file("build/reports/dependency-contract.txt")
    doLast {
        output.get().asFile.apply {
            parentFile.mkdirs()
            writeText("dependency constraints corpus\n")
        }
    }
}

tasks.named("build") {
    dependsOn(tasks.named("dependencyContractReport"))
}
