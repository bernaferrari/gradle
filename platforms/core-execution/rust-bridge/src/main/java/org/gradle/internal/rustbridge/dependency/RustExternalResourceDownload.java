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

import org.gradle.internal.buildoption.RustExternalResourceDownloadRegistry;
import org.gradle.internal.buildoption.RustExternalResourceDownloadRegistry.ExternalResourceCoordinate;

import java.io.File;
import java.net.URI;

/**
 * External resource downloader backed by the Rust dependency transport.
 */
public class RustExternalResourceDownload implements RustExternalResourceDownloadRegistry.ExternalResourceDownload {
    private final RustDependencyResolutionClient client;

    public RustExternalResourceDownload(RustDependencyResolutionClient client) {
        this.client = client;
    }

    @Override
    public boolean download(URI location, File destination) {
        return client.downloadResource(location, destination).isSuccess();
    }

    @Override
    public boolean download(URI location, File destination, ExternalResourceCoordinate coordinate) {
        if (coordinate == null) {
            return download(location, destination);
        }
        return client.downloadResource(
            location,
            destination,
            new RustDependencyResolutionClient.MavenArtifactCoordinate(
                coordinate.getGroup(),
                coordinate.getName(),
                coordinate.getVersion(),
                coordinate.getClassifier(),
                coordinate.getExtension()
            )
        ).isSuccess();
    }
}
