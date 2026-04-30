/*
 * Copyright 2026 the original author or authors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package org.gradle.internal.buildoption;

import org.jspecify.annotations.Nullable;

import java.io.File;

/**
 * Process-local hook for optional external artifact read-through providers.
 */
public final class RustArtifactCacheReadThroughRegistry {
    public interface ArtifactCacheReadThrough {
        ArtifactCacheReadThrough NO_OP = (group, name, version, classifier, extension) -> null;

        @Nullable
        File findCachedArtifact(String group, String name, String version, String classifier, String extension);
    }

    private static volatile ArtifactCacheReadThrough readThrough = ArtifactCacheReadThrough.NO_OP;

    private RustArtifactCacheReadThroughRegistry() {
    }

    public static ArtifactCacheReadThrough get() {
        return readThrough;
    }

    public static void set(ArtifactCacheReadThrough readThrough) {
        RustArtifactCacheReadThroughRegistry.readThrough = readThrough == null ? ArtifactCacheReadThrough.NO_OP : readThrough;
    }

    public static void reset() {
        readThrough = ArtifactCacheReadThrough.NO_OP;
    }
}
