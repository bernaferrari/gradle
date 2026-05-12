plugins {
    java
}

repositories {
    mavenCentral()
}

dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.2")
    testRuntimeOnly("org.junit.platform:junit-platform-console-standalone:1.10.2")
}

tasks.test {
    filter {
        includeTestsMatching("com.example.AppTest.someMethod")
    }
    useJUnitPlatform()
    outputs.dir("build/test-results/test")
}
