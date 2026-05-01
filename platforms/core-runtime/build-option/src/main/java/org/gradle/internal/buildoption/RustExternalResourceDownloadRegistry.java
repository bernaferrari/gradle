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

import java.io.File;
import java.net.URI;

/**
 * Process-local hook for optional Rust-backed external resource downloads.
 */
public final class RustExternalResourceDownloadRegistry {
    public interface ExternalResourceDownload {
        ExternalResourceDownload NO_OP = (location, destination) -> false;

        boolean download(URI location, File destination);
    }

    private static volatile ExternalResourceDownload download = ExternalResourceDownload.NO_OP;

    private RustExternalResourceDownloadRegistry() {
    }

    public static ExternalResourceDownload get() {
        return download;
    }

    public static void set(ExternalResourceDownload download) {
        RustExternalResourceDownloadRegistry.download = download == null ? ExternalResourceDownload.NO_OP : download;
    }

    public static void reset() {
        download = ExternalResourceDownload.NO_OP;
    }
}
