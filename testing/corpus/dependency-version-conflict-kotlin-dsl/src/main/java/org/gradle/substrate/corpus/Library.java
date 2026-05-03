package org.gradle.substrate.corpus;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public class Library {
    private static final Logger LOG = LoggerFactory.getLogger(Library.class);

    public void log(String message) {
        LOG.info(message);
    }
}
