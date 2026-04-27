package org.gradle.substrate.corpus.app;

import org.gradle.substrate.corpus.lib.Message;

public final class App {
    private App() {
    }

    public static void main(String[] args) {
        System.out.println(Message.value());
    }
}
