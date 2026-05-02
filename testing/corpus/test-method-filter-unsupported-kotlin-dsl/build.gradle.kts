plugins {
    java
}

tasks.test {
    filter {
        includeTestsMatching("com.example.AppTest.someMethod")
    }
    outputs.dir("build/test-results/test")
}
