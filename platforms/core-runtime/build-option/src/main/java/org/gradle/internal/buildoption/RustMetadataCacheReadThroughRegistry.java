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
import java.net.URI;

/**
 * Process-local hook for optional external metadata read-through providers.
 */
public final class RustMetadataCacheReadThroughRegistry {
    public interface MetadataCacheReadThrough {
        MetadataCacheReadThrough NO_OP = (location, extension) -> null;

        @Nullable
        File findCachedMetadata(URI location, String extension);
    }

    private static volatile MetadataCacheReadThrough readThrough = MetadataCacheReadThrough.NO_OP;

    private RustMetadataCacheReadThroughRegistry() {
    }

    public static MetadataCacheReadThrough get() {
        return readThrough;
    }

    public static void set(MetadataCacheReadThrough readThrough) {
        RustMetadataCacheReadThroughRegistry.readThrough = readThrough == null ? MetadataCacheReadThrough.NO_OP : readThrough;
    }

    public static void reset() {
        readThrough = MetadataCacheReadThrough.NO_OP;
    }
}
