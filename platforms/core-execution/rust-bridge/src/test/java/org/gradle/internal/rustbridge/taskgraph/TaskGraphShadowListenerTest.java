package org.gradle.internal.rustbridge.taskgraph;

import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import java.io.File;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class TaskGraphShadowListenerTest {

    @Rule
    public TemporaryFolder temporaryFolder = new TemporaryFolder();

    @Test
    public void stableBuildIdentityIsCanonicalRootDerived() throws Exception {
        File root = temporaryFolder.newFolder("root");
        File nested = new File(root, "nested");
        assertTrue(nested.mkdirs());

        String first = TaskGraphShadowListener.stableBuildIdentityForRoot(root);
        String second = TaskGraphShadowListener.stableBuildIdentityForRoot(new File(nested, ".."));

        assertEquals(first, second);
        assertTrue(first.startsWith("stable-root-"));
        assertEquals("stable-root-".length() + 16, first.length());
    }
}
