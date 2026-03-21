package org.gradle.internal.rustbridge.output;

import org.gradle.api.logging.Logging;
import org.gradle.internal.execution.OutputChangeListener;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

/**
 * An {@link OutputChangeListener} that shadows output cache invalidation events
 * to the Rust substrate. Fire-and-forget: never affects build correctness.
 */
public class OutputChangeShadowListener implements OutputChangeListener {

    private static final Logger LOGGER = Logging.getLogger(OutputChangeShadowListener.class);

    private final SubstrateClient client;

    public OutputChangeShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void invalidateCachesFor(Iterable<String> affectedOutputPaths) {
        if (client.isNoop()) {
            return;
        }

        try {
            int count = 0;
            StringBuilder paths = new StringBuilder();
            for (String path : affectedOutputPaths) {
                if (count > 0) paths.append(", ");
                paths.append(path);
                count++;
            }

            LOGGER.debug("[substrate:output] shadow: {} paths invalidated", count);
        } catch (Exception e) {
            LOGGER.debug("[substrate:output] shadow invalidate failed", e);
        }
    }
}
