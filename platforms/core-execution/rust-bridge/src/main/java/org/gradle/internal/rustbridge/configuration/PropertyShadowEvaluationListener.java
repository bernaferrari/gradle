package org.gradle.internal.rustbridge.configuration;

import org.gradle.api.Project;
import org.gradle.api.ProjectEvaluationListener;
import org.gradle.api.ProjectState;
import org.gradle.api.plugins.Plugin;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * A {@link ProjectEvaluationListener} that registers projects with the Rust
 * substrate after evaluation and shadow-resolves their properties.
 *
 * <p>Collects project properties and applied plugin names, then delegates to
 * {@link ShadowingPropertyResolver} for Rust shadow comparison.</p>
 */
public class PropertyShadowEvaluationListener implements ProjectEvaluationListener {

    private final ShadowingPropertyResolver propertyResolver;

    public PropertyShadowEvaluationListener(ShadowingPropertyResolver propertyResolver) {
        this.propertyResolver = propertyResolver;
    }

    @Override
    public void beforeEvaluate(Project project) {
        // No-op before evaluation
    }

    @Override
    public void afterEvaluate(Project project, ProjectState state) {
        if (state.getFailure() != null) {
            // Skip failed evaluations
            return;
        }

        try {
            // Collect project properties as strings
            Map<String, String> properties = new LinkedHashMap<>();
            Map<String, ? extends Object> rawProperties = project.getProperties();
            for (Map.Entry<String, ? extends Object> entry : rawProperties.entrySet()) {
                if (entry.getValue() != null) {
                    properties.put(entry.getKey(), entry.getValue().toString());
                }
            }

            // Collect applied plugin names
            List<String> appliedPlugins = new ArrayList<>();
            for (Plugin<?> plugin : project.getPlugins()) {
                appliedPlugins.add(plugin.getClass().getName());
            }

            // Register project with Rust and shadow-resolve properties
            propertyResolver.registerProject(
                project.getPath(),
                project.getProjectDir().getAbsolutePath(),
                properties,
                appliedPlugins
            );

            // Shadow-resolve key properties
            for (Map.Entry<String, String> entry : properties.entrySet()) {
                propertyResolver.shadowResolveProperty(
                    project.getPath(),
                    entry.getKey(),
                    entry.getValue()
                );
            }
        } catch (Exception e) {
            // Don't let shadow failures affect the build
        }
    }
}
