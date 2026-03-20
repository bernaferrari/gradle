package org.gradle.internal.rustbridge;

public class SubstrateException extends RuntimeException {
    public SubstrateException(String message) {
        super(message);
    }

    public SubstrateException(String message, Throwable cause) {
        super(message, cause);
    }
}
