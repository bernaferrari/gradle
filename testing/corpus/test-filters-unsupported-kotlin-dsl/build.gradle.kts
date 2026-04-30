plugins {
    java
}

tasks.test {
    filter {
        includeTestsMatching("com.example.*Test")
        excludeTestsMatching("com.example.Legacy*")
    }
    outputs.dir("build/test-results/test")
}
