package com.example;

import org.junit.jupiter.api.Test;

public class AppTest {
    @Test
    public void someMethod() {
    }

    @Test
    public void filteredOutMethod() {
        throw new AssertionError("filteredOutMethod should not run");
    }
}
