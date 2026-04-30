package example;

import example.internal.Helper;

/**
 * Small public API for an OSS-style corpus library.
 */
public class PublicApi {
    /**
     * Returns a stable greeting.
     *
     * @param name user name
     * @return greeting
     */
    public String greet(String name) {
        return Helper.prefix() + name;
    }
}
