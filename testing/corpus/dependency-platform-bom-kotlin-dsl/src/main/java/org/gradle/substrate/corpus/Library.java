package org.gradle.substrate.corpus;

import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.logging.HttpLoggingInterceptor;

import java.io.IOException;

public class Library {
    private final OkHttpClient client;

    public Library() {
        HttpLoggingInterceptor logger = new HttpLoggingInterceptor();
        logger.setLevel(HttpLoggingInterceptor.Level.BASIC);
        this.client = new OkHttpClient.Builder()
            .addInterceptor(logger)
            .build();
    }

    public OkHttpClient getClient() {
        return client;
    }
}
