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

package org.gradle.internal.rustbridge.dependency;

import org.gradle.internal.buildoption.RustArtifactCacheReadThroughRegistry;

import java.io.File;

/**
 * Read-through source backed by the Rust artifact store.
 */
public class RustArtifactCacheReadThrough implements RustArtifactCacheReadThroughRegistry.ArtifactCacheReadThrough {
    private final RustDependencyResolutionClient client;

    public RustArtifactCacheReadThrough(RustDependencyResolutionClient client) {
        this.client = client;
    }

    @Override
    public File findCachedArtifact(String group, String name, String version, String classifier, String extension) {
        RustDependencyResolutionClient.CacheCheckResult result = client.checkArtifactCache(
            group,
            name,
            version,
            classifier,
            extension,
            ""
        );
        if (!result.isCached() || result.getLocalPath() == null || result.getLocalPath().isEmpty()) {
            return null;
        }

        File file = new File(result.getLocalPath());
        return file.isFile() ? file : null;
    }
}
