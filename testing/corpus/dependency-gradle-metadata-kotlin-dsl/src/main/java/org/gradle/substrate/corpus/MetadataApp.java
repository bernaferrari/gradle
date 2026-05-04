package org.gradle.substrate.corpus;

import okhttp3.Request;

public final class MetadataApp {
    private MetadataApp() {
    }

    public static String method() {
        return new Request.Builder()
            .url("https://example.invalid/")
            .build()
            .method();
    }
}
