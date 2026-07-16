package org.gradle.substrate.corpus.kotlinlib

import kotlin.test.Test
import kotlin.test.assertEquals

class LibraryTest {
    @Test
    fun message() {
        assertEquals("kotlin-jvm-library", Library().message())
    }
}
