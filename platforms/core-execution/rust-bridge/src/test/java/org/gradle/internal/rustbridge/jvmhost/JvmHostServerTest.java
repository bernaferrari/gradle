package org.gradle.internal.rustbridge.jvmhost;

import org.junit.Test;

import java.util.List;

import static org.junit.Assert.*;

/**
 * Unit tests for {@link JvmHostServer#extractPlugins(String)}.
 */
public class JvmHostServerTest {

    @Test
    public void extractKotlinDslPlugins() {
        String script = """
            plugins {
                id("java")
                id("org.springframework.boot") version "3.2.0"
                id("io.spring.dependency-management") version "1.1.4"
            }
            """;

        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins(script);

        assertEquals(3, plugins.size());
        assertEquals("java", plugins.get(0).getPluginId());
        assertEquals("org.springframework.boot", plugins.get(1).getPluginId());
        assertEquals("io.spring.dependency-management", plugins.get(2).getPluginId());
        assertEquals("1", plugins.get(0).getApplyOrder());
        assertEquals("2", plugins.get(1).getApplyOrder());
        assertEquals("3", plugins.get(2).getApplyOrder());
    }

    @Test
    public void extractGroovyDslPlugins() {
        String script = """
            plugins {
                id 'java'
                id 'org.springframework.boot' version '3.2.0'
            }
            """;

        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins(script);

        assertEquals(2, plugins.size());
        assertEquals("java", plugins.get(0).getPluginId());
        assertEquals("org.springframework.boot", plugins.get(1).getPluginId());
    }

    @Test
    public void extractLegacyApplyPlugin() {
        String script = """
            apply plugin: 'java'
            apply plugin: 'groovy'
            apply plugin: 'idea'
            """;

        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins(script);

        assertEquals(3, plugins.size());
        assertEquals("java", plugins.get(0).getPluginId());
        assertEquals("groovy", plugins.get(1).getPluginId());
        assertEquals("idea", plugins.get(2).getPluginId());
    }

    @Test
    public void extractKotlinJvmPlugin() {
        String script = """
            plugins {
                kotlin("jvm")
                kotlin("js") version "1.9.20"
            }
            """;

        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins(script);

        assertEquals(2, plugins.size());
        assertEquals("org.jetbrains.kotlin.jvm", plugins.get(0).getPluginId());
        assertEquals("org.jetbrains.kotlin.js", plugins.get(1).getPluginId());
    }

    @Test
    public void extractMixedPluginsMaintainsOrder() {
        String script = """
            plugins {
                id("java")
                kotlin("jvm")
                id("org.springframework.boot")
            }
            apply plugin: 'idea'
            """;

        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins(script);

        assertEquals(4, plugins.size());
        assertEquals("java", plugins.get(0).getPluginId());
        assertEquals("org.jetbrains.kotlin.jvm", plugins.get(1).getPluginId());
        assertEquals("org.springframework.boot", plugins.get(2).getPluginId());
        assertEquals("idea", plugins.get(3).getPluginId());
    }

    @Test
    public void extractEmptyScript() {
        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins("");
        assertTrue(plugins.isEmpty());
    }

    @Test
    public void extractNoPlugins() {
        String script = """
            repositories {
                mavenCentral()
            }
            dependencies {
                implementation("org.example:lib:1.0")
            }
            """;

        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins(script);
        assertTrue(plugins.isEmpty());
    }

    @Test
    public void deduplicatePlugins() {
        String script = """
            plugins {
                id("java")
                id("java")
                id("org.springframework.boot")
                id("org.springframework.boot")
            }
            """;

        List<gradle.substrate.v1.AppliedPlugin> plugins = JvmHostServer.extractPlugins(script);

        assertEquals(2, plugins.size());
        assertEquals("java", plugins.get(0).getPluginId());
        assertEquals("org.springframework.boot", plugins.get(1).getPluginId());
    }
}
