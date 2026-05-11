package org.gradle.substrate.corpus.composite;

import org.gradle.substrate.included.IncludedLib;

public class CompositeConsumer {
    public String value() {
        return new IncludedLib().value();
    }
}
