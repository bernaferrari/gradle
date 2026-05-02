package org.gradle.substrate.corpus.junit;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.fail;

class LegacySlowTest {
    @Test
    void mustBeExcludedByClassFilter() {
        fail("LegacySlowTest should be excluded by the native TestExec class filter");
    }
}
