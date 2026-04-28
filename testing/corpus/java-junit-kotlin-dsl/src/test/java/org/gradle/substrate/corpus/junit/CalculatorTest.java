package org.gradle.substrate.corpus.junit;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Tag;

import static org.junit.jupiter.api.Assertions.assertEquals;

class CalculatorTest {
    @Test
    @Tag("fast")
    void addsNumbers() {
        assertEquals(5, Calculator.add(2, 3));
    }

    @Test
    @Tag("slow")
    void skippedByTagFilter() {
        assertEquals(0, Calculator.add(1, -1));
    }
}
