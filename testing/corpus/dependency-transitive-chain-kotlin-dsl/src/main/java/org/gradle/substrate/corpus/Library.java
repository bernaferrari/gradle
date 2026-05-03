package org.gradle.substrate.corpus;

import org.apache.commons.compress.compressors.gzip.GzipCompressorInputStream;
import com.google.gson.Gson;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;

public class Library {
    private final Gson gson = new Gson();

    public String toJson(Object obj) {
        return gson.toJson(obj);
    }

    public boolean isGzipMagic(byte[] data) {
        return data.length >= 2
            && (data[0] & 0xff) == 0x1f
            && (data[1] & 0xff) == 0x8b;
    }
}
