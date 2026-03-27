use std::path::Path;

// Re-export all IR types from the shared types module.
pub use super::build_script_types::*;

/// Parse a Gradle build script (Kotlin DSL or Groovy) and extract
/// plugins, dependencies, task configs, repositories, and subprojects.
///
/// Tries AST-based extraction first (structured, with line numbers).
/// Falls back to string-based extraction when the parser reports errors.
pub fn parse_build_script(content: &str, file_name: &str) -> BuildScriptParseResult {
    let script_type = detect_script_type(file_name, content);

    // Try AST-based extraction first — it provides line numbers and
    // structurally robust results.
    let ast_result = crate::server::groovy_parser::parse(content);
    if ast_result.errors.is_empty() {
        let mut result = super::ast_extractor::extract_from_ast(&ast_result.script, script_type);
        result.script_type = script_type;

        // If the AST extractor found nothing but the content clearly has
        // blocks, the parser likely consumed multi-line constructs as
        // single expressions. Fall back to string-based extraction.
        // Only consider "substantial" elements — settings scripts with just
        // subprojects/include should fall through to string-based extraction
        // which handles pluginManagement/dependencyResolutionManagement.
        let has_elements = !result.plugins.is_empty()
            || !result.dependencies.is_empty()
            || !result.repositories.is_empty()
            || !result.task_configs.is_empty();

        // If the parser consumed the entire script as a single statement,
        // the AST extraction is likely incomplete (parser's no-paren
        // greediness merged multiple top-level blocks). Fall back.
        let single_statement = ast_result.script.statements.len() <= 1;
        let multi_block = content.matches('{').count() > 1;

        if has_elements && !(single_statement && multi_block) || !content.contains('{') {
            if script_type == ScriptType::Unknown {
                result
                    .warnings
                    .push("Unknown build script type".to_string());
            }
            return result;
        }
        // Fall through to string-based extraction
    }

    // Fallback: string-based extraction for scripts that the parser
    // cannot fully handle (partial parse / error recovery).
    match script_type {
        ScriptType::KotlinDsl => parse_kotlin_dsl(content),
        ScriptType::Groovy => parse_groovy(content),
        ScriptType::Unknown => {
            let mut result = BuildScriptParseResult::default();
            result
                .warnings
                .push("Unknown build script type".to_string());
            result
        }
    }
}

/// Detect the script type from file name and content.
fn detect_script_type(file_name: &str, content: &str) -> ScriptType {
    // Strong extension signals take priority
    if file_name.ends_with(".kts") {
        return ScriptType::KotlinDsl;
    }
    if file_name.ends_with(".gradle") {
        // Content heuristic can override .gradle extension
        // (some projects use .gradle for Kotlin DSL)
        if content.contains("plugins {") && content.contains("id(\"") {
            return ScriptType::KotlinDsl;
        }
        return ScriptType::Groovy;
    }
    // Heuristic: if content contains "plugins { id(" it's likely Kotlin DSL
    if content.contains("plugins {") && content.contains("id(\"") {
        ScriptType::KotlinDsl
    } else if content.contains("plugins {") || content.contains("apply plugin:") {
        ScriptType::Groovy
    } else {
        ScriptType::Unknown
    }
}

/// Parse a Kotlin DSL build script.
fn parse_kotlin_dsl(content: &str) -> BuildScriptParseResult {
    let mut result = BuildScriptParseResult {
        script_type: ScriptType::KotlinDsl,
        ..BuildScriptParseResult::default()
    };

    // Remove block comments /* ... */
    let content_no_comments = remove_block_comments(content);
    // Remove line comments // ...
    let content_clean = remove_line_comments(&content_no_comments);

    // Parse plugins block: plugins { id("foo") apply false }
    parse_plugins_block(&content_clean, &mut result);

    // Parse dependencies block: dependencies { implementation("...") }
    parse_dependencies_block(&content_clean, &mut result);

    // Parse version catalog references: implementation(libs.commons.lang3)
    parse_version_catalog_refs(&content_clean, &mut result);

    // Parse buildscript block: buildscript { classpath("...") }
    parse_buildscript_block(&content_clean, &mut result);

    // Parse repositories block: repositories { mavenCentral() }
    parse_repositories_block(&content_clean, &mut result);

    // Parse task configurations: tasks.register("foo") { dependsOn("bar") }
    parse_tasks_block(&content_clean, &mut result);

    // Parse top-level assignments
    parse_top_level_assignments(&content_clean, &mut result);

    // Parse subproject includes
    parse_subproject_includes(&content_clean, &mut result);

    // Parse pluginManagement block (settings.gradle.kts)
    parse_plugin_management(&content_clean, &mut result);

    // Parse dependencyResolutionManagement block (settings.gradle.kts)
    parse_dependency_resolution_management(&content_clean, &mut result);

    result
}

/// Parse a Groovy build script.
fn parse_groovy(content: &str) -> BuildScriptParseResult {
    let mut result = BuildScriptParseResult {
        script_type: ScriptType::Groovy,
        ..BuildScriptParseResult::default()
    };

    let content_no_comments = remove_block_comments(content);
    let content_clean = remove_line_comments(&content_no_comments);

    // Parse plugins { id "foo" }
    parse_groovy_plugins(&content_clean, &mut result);

    // Parse dependencies { implementation '...' }
    parse_groovy_dependencies(&content_clean, &mut result);

    // Parse version catalog references
    parse_version_catalog_refs(&content_clean, &mut result);

    // Parse buildscript block
    parse_buildscript_block(&content_clean, &mut result);

    // Parse repositories { mavenCentral() }
    parse_repositories_block(&content_clean, &mut result);

    // Parse task configurations
    parse_groovy_tasks(&content_clean, &mut result);

    // Parse top-level assignments
    parse_top_level_assignments(&content_clean, &mut result);

    // Parse subproject includes
    parse_subproject_includes(&content_clean, &mut result);

    // Parse pluginManagement block (settings.gradle.kts)
    parse_plugin_management(&content_clean, &mut result);

    // Parse dependencyResolutionManagement block (settings.gradle.kts)
    parse_dependency_resolution_management(&content_clean, &mut result);

    result
}

/// Remove /* ... */ block comments.
fn remove_block_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next(); // consume '*'
                          // Preserve line structure: only emit newlines for lines the comment spans
            loop {
                match chars.next() {
                    Some('*') if chars.peek() == Some(&'/') => {
                        chars.next(); // consume '/'
                        break;
                    }
                    Some('\n') => result.push('\n'),
                    Some(_) => {}
                    None => break,
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Remove // line comments (but not inside strings).
fn remove_line_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut in_string = false;
    let mut string_char = ' ';

    for line in content.lines() {
        let mut line_result = String::with_capacity(line.len());

        for (i, c) in line.char_indices() {
            if in_string {
                line_result.push(c);
                if c == string_char && !is_escaped(line, i) {
                    in_string = false;
                }
            } else if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
                line_result.push(c);
            } else if c == '/' && line.chars().nth(i + 1) == Some('/') {
                break; // Rest of line is comment
            } else {
                line_result.push(c);
            }
        }

        result.push_str(&line_result);
        result.push('\n');
    }

    // Remove trailing newline if original didn't end with one
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.truncate(result.len() - 1);
    }

    result
}

/// Check if character at index is escaped with backslash.
fn is_escaped(content: &str, index: usize) -> bool {
    if index == 0 {
        return false;
    }
    let mut count = 0;
    let mut pos = index - 1;
    let chars: Vec<char> = content.chars().collect();
    while pos > 0 && chars.get(pos) == Some(&'\\') {
        count += 1;
        pos -= 1;
    }
    count % 2 == 1
}

/// Extract a string literal from a Kotlin DSL or Groovy string argument.
/// Handles both `"..."` and `'...'`.
fn extract_string_literal(s: &str) -> Option<String> {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

/// Find the content between matching braces.
fn find_brace_block(content: &str, start: usize) -> Option<String> {
    let bytes = content.as_bytes();
    if start >= bytes.len() || bytes[start] != b'{' {
        return None;
    }

    let mut depth = 1;
    let mut i = start + 1;
    let mut in_string = false;
    let mut string_char = b' ';

    while i < bytes.len() && depth > 0 {
        let c = bytes[i];

        if in_string {
            if c == string_char && !is_escaped(content, i) {
                in_string = false;
            }
        } else if c == b'"' || c == b'\'' {
            in_string = true;
            string_char = c;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
        }

        if depth > 0 {
            i += 1;
        }
    }

    if depth == 0 {
        Some(content[start + 1..i].to_string())
    } else {
        None
    }
}

/// Find the position of the matching closing parenthesis.
/// `content` should start right after the opening `(`.
fn find_matching_paren(content: &str) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut depth = 1;
    let mut i = 0;
    let mut in_string = false;
    let mut string_char = b' ';

    while i < bytes.len() && depth > 0 {
        let c = bytes[i];
        if in_string {
            if c == string_char && !is_escaped(content, i) {
                in_string = false;
            }
        } else if c == b'"' || c == b'\'' {
            in_string = true;
            string_char = c;
        } else if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
        }
        if depth > 0 {
            i += 1;
        }
    }

    if depth == 0 {
        Some(i)
    } else {
        None
    }
}

/// Find the position of a top-level block: `keyword { ... }`
fn find_top_level_block(content: &str, keyword: &str) -> Option<(usize, String)> {
    // Look for `keyword` followed by whitespace and `{`
    let search = format!("{} ", keyword);
    let pos = match content.find(&search) {
        Some(p) => Some(p),
        None => {
            // Try keyword immediately followed by `{`
            let alt = format!("{}{{", keyword);
            content.find(&alt)
        }
    };

    let pos = pos?;
    let after_keyword = &content[pos + keyword.len()..];

    // Skip whitespace to find `{`
    let brace_pos = after_keyword.find('{')?;

    let abs_brace = pos + keyword.len() + brace_pos;
    let block = find_brace_block(content, abs_brace)?;
    Some((abs_brace, block))
}

/// Parse plugins block in Kotlin DSL.
fn parse_plugins_block(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "plugins") {
        // Find all id("...") calls
        for (i, _) in block.match_indices("id(") {
            let args_start = i + 3;
            if let Some(close) = block[args_start..].find(')') {
                let args = &block[args_start..args_start + close];
                if let Some(id) = extract_string_literal(args) {
                    // Check for "apply false" only within this plugin's statement
                    // (from this id( to the next id( or end of block)
                    let statement_end = block[i + 3..]
                        .find("id(")
                        .map(|j| i + 3 + j)
                        .unwrap_or(block.len());
                    let statement = &block[i..statement_end];
                    let apply = !statement.contains("apply false");
                    result.plugins.push(ParsedPlugin { id, apply, ..Default::default() });
                }
            }
        }
    }

    // Also parse standalone `apply(plugin = "...")` or `apply(plugin: "...")`
    for (i, _) in content.match_indices("apply(") {
        let args_start = i + 6;
        if let Some(close) = content[args_start..].find(')') {
            let args = &content[args_start..args_start + close];
            // Kotlin DSL: apply(plugin = "foo")
            if let Some(eq_pos) = args.find('=') {
                let value = args[eq_pos + 1..].trim();
                if let Some(id) = extract_string_literal(value) {
                    result.plugins.push(ParsedPlugin { id, apply: true, ..Default::default() });
                }
            }
        }
    }
}

/// Parse dependencies block in Kotlin DSL.
fn parse_dependencies_block(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "dependencies") {
        // Kotlin DSL: implementation("com.example:lib:1.0")
        // Also handles: testImplementation, api, compileOnly, runtimeOnly, annotationProcessor
        let config_keywords = [
            "implementation",
            "api",
            "compileOnly",
            "runtimeOnly",
            "testImplementation",
            "testRuntimeOnly",
            "testCompileOnly",
            "androidTestImplementation",
            "annotationProcessor",
            "kapt",
            "kaptTest",
        ];

        // Collect dependencies with their source positions for ordering
        let mut found_deps: Vec<(usize, ParsedDependency)> = Vec::new();

        for kw in &config_keywords {
            let pattern = format!("{}(", kw);
            let mut search_from = 0;
            while let Some(i) = block[search_from..].find(&pattern) {
                let abs_i = search_from + i;
                // Ensure the match is not part of a longer identifier
                if abs_i > 0
                    && block
                        .as_bytes()
                        .get(abs_i - 1)
                        .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    search_from = abs_i + 1;
                    continue;
                }
                let args_start = abs_i + pattern.len();
                if let Some(close) = find_matching_paren(&block[args_start..]) {
                    let args = &block[args_start..args_start + close];
                    if let Some(notation) = extract_string_literal(args) {
                        found_deps.push((
                            abs_i,
                            ParsedDependency {
                                configuration: kw.to_string(),
                                notation,
                                ..Default::default()
                            },
                        ));
                    } else if args.trim().starts_with("project(") {
                        found_deps.push((
                            abs_i,
                            ParsedDependency {
                                configuration: kw.to_string(),
                                notation: args.trim().to_string(),
                                ..Default::default()
                            },
                        ));
                    }
                }
                search_from = abs_i + 1;
            }
        }

        // Sort by source position to maintain declaration order
        found_deps.sort_unstable_by_key(|(pos, _)| *pos);
        result
            .dependencies
            .extend(found_deps.into_iter().map(|(_, dep)| dep));
    }

    // Parse standalone dependency declarations outside a block:
    // val implementation by platform.deps(...)
    for kw in &["implementation", "api", "compileOnly", "runtimeOnly"] {
        let pattern = format!("val {} by ", kw);
        if let Some(_i) = content.find(&pattern) {
            result.warnings.push(format!(
                "Version catalog dependency '{}' not fully parsed",
                kw
            ));
        }
    }
}

/// Parse version catalog references inside a dependencies block.
///
/// Handles:
/// - `implementation(libs.commons.lang3)` — Kotlin DSL
/// - `implementation(libs.versions.java.get())` — version reference
/// - `testImplementation(platform(libs.androidx.test)))` — wrapped accessors
/// - `implementation libs.commons.lang3` — Groovy DSL (no parens)
fn parse_version_catalog_refs(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "dependencies") {
        let config_keywords = [
            "implementation",
            "api",
            "compileOnly",
            "runtimeOnly",
            "testImplementation",
            "testRuntimeOnly",
            "testCompileOnly",
            "androidTestImplementation",
            "kapt",
            "kaptTest",
        ];

        for kw in &config_keywords {
            // Kotlin DSL pattern: config(libs.something)
            let pattern = format!("{}(", kw);
            let mut search_from = 0;
            while let Some(i) = block[search_from..].find(&pattern) {
                let abs_i = search_from + i;

                // Ensure not part of a longer identifier
                if abs_i > 0
                    && block
                        .as_bytes()
                        .get(abs_i - 1)
                        .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    search_from = abs_i + 1;
                    continue;
                }

                let args_start = abs_i + pattern.len();
                if let Some(close) = find_matching_paren(&block[args_start..]) {
                    let args = block[args_start..args_start + close].trim();

                    // Check if this is a catalog reference (contains "libs.")
                    if args.contains("libs.") {
                        // Extract the alias — strip platform(...) wrapper if present
                        let alias = if args.starts_with("platform(")
                            && args.ends_with(')')
                        {
                            &args[9..args.len() - 1]
                        } else {
                            args
                        };

                        // Strip trailing .get(), .asProvider(), etc.
                        let clean_alias = alias
                            .trim_end_matches(".get()")
                            .trim_end_matches(".asProvider()")
                            .trim_end_matches(")")
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'');

                        if !clean_alias.is_empty() && clean_alias.contains('.') {
                            result.catalog_refs.push(ParsedVersionCatalogRef {
                                configuration: kw.to_string(),
                                alias: clean_alias.to_string(),
                            });
                        }
                    }
                }
                search_from = abs_i + 1;
            }

            // Groovy DSL pattern: config libs.something (no parens)
            let space_pattern = format!("{} ", kw);
            let mut search_from = 0;
            while let Some(i) = block[search_from..].find(&space_pattern) {
                let abs_i = search_from + i;

                // Ensure not part of a longer identifier
                if abs_i > 0
                    && block
                        .as_bytes()
                        .get(abs_i - 1)
                        .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    search_from = abs_i + 1;
                    continue;
                }

                let rest = block[abs_i + space_pattern.len()..].trim();
                // Check if this starts with libs. and extract the alias
                if let Some(alias) = rest.split_whitespace().next() {
                    if alias.starts_with("libs.") {
                        let clean_alias = alias
                            .trim_end_matches(".get()")
                            .trim_end_matches(".asProvider()")
                            .trim_end_matches(')')
                            .trim_end_matches('(')
                            .trim();

                        if !clean_alias.is_empty() && clean_alias.contains('.') {
                            result.catalog_refs.push(ParsedVersionCatalogRef {
                                configuration: kw.to_string(),
                                alias: clean_alias.to_string(),
                            });
                        }
                    }
                }
                search_from = abs_i + 1;
            }
        }
    }
}

/// Parse the `buildscript` block to extract classpath dependencies.
///
/// Handles:
/// ```kotlin
/// buildscript {
///     dependencies {
///         classpath("com.example:plugin:1.0")
///     }
/// }
/// ```
fn parse_buildscript_block(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "buildscript") {
        // Search for classpath directly in the buildscript block
        // (avoids find_top_level_block which would match nested "dependencies")
        // Kotlin: classpath("group:artifact:ver")   Groovy: classpath 'group:artifact:ver'
        let patterns = ["classpath(", "classpath '", "classpath \""];

        for pat in &patterns {
            let mut search_from = 0;
            while let Some(i) = block[search_from..].find(pat) {
                let abs_i = search_from + i;

                // Ensure not part of a longer identifier
                if abs_i > 0
                    && block
                        .as_bytes()
                        .get(abs_i - 1)
                        .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    search_from = abs_i + 1;
                    continue;
                }

                let args_start = abs_i + pat.len();

                if pat.ends_with('(') {
                    // Kotlin paren form: classpath("...")
                    if let Some(close) = find_matching_paren(&block[args_start..]) {
                        let args = block[args_start..args_start + close].trim();
                        if let Some(notation) = extract_string_literal(args) {
                            result.buildscript_deps.push(ParsedBuildScriptDep {
                                notation,
                            });
                        }
                    }
                } else {
                    // Groovy no-paren form: classpath '...'
                    // Pattern consumed the opening quote, so args starts with the notation
                    let rest = &block[args_start..];
                    let line_end = rest
                        .find('\n')
                        .unwrap_or(rest.len());
                    let args = rest[..line_end].trim();
                    // The opening quote was consumed by the pattern; strip trailing quote
                    let notation = args
                        .strip_suffix('\'')
                        .or_else(|| args.strip_suffix('"'))
                        .unwrap_or(args);
                    if !notation.is_empty() {
                        result.buildscript_deps.push(ParsedBuildScriptDep {
                            notation: notation.to_string(),
                        });
                    }
                }

                search_from = abs_i + 1;
            }
        }
    }
}

/// Parse `pluginManagement` block from settings.gradle(.kts).
///
/// Handles:
/// ```kotlin
/// pluginManagement {
///     repositories {
///         gradlePluginPortal()
///         maven { url = uri("https://...") }
///         mavenCentral()
///     }
/// }
/// ```
fn parse_plugin_management(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "pluginManagement") {
        let mut pm = ParsedPluginManagement::default();

        // Find the nested repositories block within pluginManagement
        if let Some((_repo_pos, repo_block)) = find_top_level_block(&block, "repositories") {
            // Collect all repos with source positions for ordering
            let mut found_repos: Vec<(usize, ParsedPluginRepository)> = Vec::new();

            // Standard shorthand repos
            let standard_repos = [
                ("gradlePluginPortal()", "gradlePluginPortal", "gradlePluginPortal"),
                ("mavenCentral()", "mavenCentral", "maven"),
                ("google()", "google", "maven"),
                ("mavenLocal()", "mavenLocal", "maven-local"),
            ];

            for (pattern, name, repo_type) in &standard_repos {
                if let Some(pos) = repo_block.find(pattern) {
                    found_repos.push((
                        pos,
                        ParsedPluginRepository {
                            name: name.to_string(),
                            repo_type: repo_type.to_string(),
                        },
                    ));
                }
            }

            // Custom maven repos: maven { url = uri("...") } or maven { url = "..." }
            for (i, _) in repo_block.match_indices("maven {") {
                let sub = &repo_block[i..];
                let brace_pos = sub.find('{').unwrap_or(sub.len());
                let inner = &sub[brace_pos + 1..];

                let close_brace = match find_brace_block_end(inner) {
                    Some(c) => c,
                    None => continue,
                };
                let body = &inner[..close_brace];

                // Try url = uri("...") or url = "..." or url("...")
                if let Some(url_pos) = body.find("url") {
                    let after_url = body[url_pos + 3..].trim();
                    let url = if let Some(eq_pos) = after_url.find('=') {
                        let value = after_url[eq_pos + 1..].trim();
                        // Handle uri("...") wrapper
                        if let Some(rest) = value.strip_prefix("uri(") {
                            if let Some(close) = find_matching_paren(rest) {
                                extract_string_literal(&rest[..close])
                            } else {
                                None
                            }
                        } else {
                            extract_string_literal(value)
                        }
                    } else if let Some(rest) = after_url.strip_prefix('(') {
                        if let Some(_close) = find_matching_paren(rest) {
                            extract_string_literal(rest)
                        } else {
                            None
                        }
                    } else {
                        // Groovy: url "..." or url '...'
                        extract_string_literal(after_url)
                    };

                    if let Some(url) = url {
                        found_repos.push((
                            i,
                            ParsedPluginRepository {
                                name: url.clone(),
                                repo_type: "maven".to_string(),
                            },
                        ));
                    }
                }
            }

            // Sort by source position to maintain declaration order
            found_repos.sort_unstable_by_key(|(pos, _)| *pos);
            for (_, repo) in found_repos {
                pm.repositories.push(repo);
            }
        }

        result.plugin_management = Some(pm);
    }
}

/// Parse `dependencyResolutionManagement` block from settings.gradle(.kts).
///
/// Handles:
/// ```kotlin
/// dependencyResolutionManagement {
///     repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
///     repositories {
///         mavenCentral()
///     }
/// }
/// ```
fn parse_dependency_resolution_management(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "dependencyResolutionManagement") {
        let mut drm = ParsedDependencyResolutionManagement::default();

        // Extract repositoriesMode
        if let Some((i, _)) = block.match_indices("repositoriesMode").next() {
            let rest = &block[i + "repositoriesMode".len()..].trim_start();
            // Kotlin: repositoriesMode.set(RepositoriesMode.XXX) or repositoriesMode.set(XXX)
            if let Some(set_pos) = rest.find(".set(") {
                let args_start = set_pos + 5;
                if let Some(close) = find_matching_paren(&rest[args_start..]) {
                    let args = rest[args_start..args_start + close].trim();
                    // Handle RepositoriesMode.PREFER_SETTINGS or just PREFER_SETTINGS
                    let mode = if let Some(dot) = args.rfind('.') {
                        &args[dot + 1..]
                    } else {
                        args
                    };
                    drm.repositories_mode = Some(mode.trim_end_matches(')').to_string());
                }
            }
            // Groovy: repositoriesMode = RepositoriesMode.XXX
            else if let Some(eq_pos) = rest.find('=') {
                let value = rest[eq_pos + 1..].trim();
                // Trim to end of line
                let value = if let Some(nl) = value.find('\n') {
                    &value[..nl]
                } else {
                    value
                }
                .trim();
                let mode = if let Some(dot) = value.rfind('.') {
                    &value[dot + 1..]
                } else {
                    value
                };
                drm.repositories_mode = Some(mode.trim_end_matches(')').to_string());
            }
        }

        // Extract repositories from the nested repositories block
        if let Some((_repo_pos, repo_block)) = find_top_level_block(&block, "repositories") {
            let standard_repos = [
                ("mavenCentral()", "mavenCentral", "maven"),
                ("google()", "google", "maven"),
                ("mavenLocal()", "mavenLocal", "maven-local"),
                ("gradlePluginPortal()", "gradlePluginPortal", "gradlePluginPortal"),
            ];

            for (pattern, name, repo_type) in &standard_repos {
                if repo_block.contains(pattern) {
                    drm.repositories.push(ParsedRepository {
                        name: name.to_string(),
                        repo_type: repo_type.to_string(),
                    });
                }
            }
        }

        result.dependency_resolution_management = Some(drm);
    }
}

/// Parse Groovy-style plugins block.
fn parse_groovy_plugins(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "plugins") {
        // Groovy: id "foo" or id 'foo'
        for (i, _) in block.match_indices("id ") {
            let args_start = i + 3;
            // Get the rest of the line (or rest of block if no newline)
            let line_end = block[args_start..]
                .find('\n')
                .unwrap_or(block.len() - args_start);
            let args = block[args_start..args_start + line_end].trim();
            if let Some(id) = extract_string_literal(args) {
                // Check if "apply false" appears on the SAME line as this id
                let rest_of_line = &block[i..args_start + line_end];
                let apply = !rest_of_line.contains("apply false");
                result.plugins.push(ParsedPlugin { id, apply, ..Default::default() });
            }
        }
    }

    // Groovy standalone: apply plugin: "foo" or apply plugin: 'foo'
    // Single pass handles both quote styles via extract_string_literal
    for (i, _) in content.match_indices("apply plugin:") {
        let args_start = i + 14;
        let line_end = content[args_start..]
            .find('\n')
            .unwrap_or(content.len() - args_start);
        let args = content[args_start..args_start + line_end].trim();
        if let Some(id) = extract_string_literal(args) {
            result.plugins.push(ParsedPlugin { id, apply: true, ..Default::default() });
        }
    }
}

/// Parse Groovy-style dependencies block.
fn parse_groovy_dependencies(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "dependencies") {
        #[cfg(test)]
        eprintln!(
            "[DEBUG parse_groovy_deps] block len={}, first 80: {:?}",
            block.len(),
            &block[..std::cmp::min(block.len(), 80)]
        );

        let config_keywords = [
            "implementation",
            "api",
            "compileOnly",
            "runtimeOnly",
            "testImplementation",
            "testRuntimeOnly",
            "testCompileOnly",
            "annotationProcessor",
        ];

        // Collect dependencies with their source positions for ordering
        let mut found_deps: Vec<(usize, ParsedDependency)> = Vec::new();

        for kw in &config_keywords {
            // Groovy: implementation 'com.example:lib:1.0'
            let single_quote = format!("{} '", kw);
            // Groovy: implementation "com.example:lib:1.0"
            let double_quote = format!("{} \"", kw);
            // Groovy: implementation("com.example:lib:1.0") or implementation('...')
            let paren_form = format!("{}(", kw);
            let mut search_from = 0;

            #[cfg(test)]
            eprintln!(
                "[DEBUG] kw={}, single_quote={:?}, paren_form={:?}",
                kw, single_quote, paren_form
            );

            while search_from < block.len() {
                // Try single-quote form first: implementation 'group:artifact:ver'
                let found = if let Some(i) = block[search_from..].find(&single_quote) {
                    let abs_i = search_from + i;
                    // Ensure not part of a longer identifier
                    if abs_i > 0
                        && block
                            .as_bytes()
                            .get(abs_i - 1)
                            .is_some_and(|c| c.is_ascii_alphabetic())
                    {
                        search_from = abs_i + 1;
                        continue;
                    }
                    let val_start = abs_i + single_quote.len();
                    if let Some(end) = block[val_start..].find('\'') {
                        let notation = block[val_start..val_start + end].to_string();
                        found_deps.push((
                            abs_i,
                            ParsedDependency {
                                configuration: kw.to_string(),
                                notation,
                                ..Default::default()
                            },
                        ));
                        search_from = val_start + end + 1;
                        continue;
                    }
                    abs_i
                } else {
                    // single_quote not found — try double-quote form from current position
                    if let Some(i) = block[search_from..].find(&double_quote) {
                        let abs_i = search_from + i;
                        if abs_i > 0
                            && block
                                .as_bytes()
                                .get(abs_i - 1)
                                .is_some_and(|c| c.is_ascii_alphabetic())
                        {
                            search_from = abs_i + 1;
                            continue;
                        }
                        let val_start = abs_i + double_quote.len();
                        if let Some(end) = block[val_start..].find('"') {
                            let notation = block[val_start..val_start + end].to_string();
                            found_deps.push((
                                abs_i,
                                ParsedDependency {
                                    configuration: kw.to_string(),
                                    notation,
                                    ..Default::default()
                                },
                            ));
                            search_from = val_start + end + 1;
                            continue;
                        }
                        abs_i
                    } else {
                        search_from
                    }
                };

                // Try paren form: implementation("...") or implementation('...')
                if let Some(i) = block[found..].find(&paren_form) {
                    let abs_i = found + i;
                    // Ensure not part of a longer identifier
                    if abs_i > 0
                        && block
                            .as_bytes()
                            .get(abs_i - 1)
                            .is_some_and(|c| c.is_ascii_alphabetic())
                    {
                        search_from = abs_i + 1;
                        continue;
                    }
                    let args_start = abs_i + paren_form.len();
                    // Use find_matching_paren to handle nested parens like exclude(group: 'x', module: 'y')
                    if let Some(close) = find_matching_paren(&block[args_start..]) {
                        let args = &block[args_start..args_start + close];
                        if let Some(notation) = extract_string_literal(args) {
                            found_deps.push((
                                abs_i,
                                ParsedDependency {
                                    configuration: kw.to_string(),
                                    notation,
                                    ..Default::default()
                                },
                            ));
                        }
                        search_from = abs_i + close + paren_form.len() + 1;
                        continue;
                    }
                }

                search_from = block.len();
            }
        }

        // Sort by source position to maintain declaration order
        found_deps.sort_unstable_by_key(|(pos, _)| *pos);
        result
            .dependencies
            .extend(found_deps.into_iter().map(|(_, dep)| dep));
    }
}

/// Parse repositories block (works for both Kotlin DSL and Groovy).
fn parse_repositories_block(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "repositories") {
        // Parse all repos in source order to maintain declaration order.
        let mut found_repos: Vec<(usize, ParsedRepository)> = Vec::new();

        // Standard shorthand repos: match whole-line forms
        let standard_repos = [
            ("mavenCentral()", "mavenCentral", "maven"),
            ("google()", "google", "maven"),
            (
                "gradlePluginPortal()",
                "gradlePluginPortal",
                "gradlePluginPortal",
            ),
            ("mavenLocal()", "mavenLocal", "maven-local"),
        ];

        for (pattern, name, repo_type) in &standard_repos {
            if let Some(pos) = block.find(pattern) {
                found_repos.push((
                    pos,
                    ParsedRepository {
                        name: name.to_string(),
                        repo_type: repo_type.to_string(),
                    },
                ));
            }
        }

        // Custom maven repos: maven { url = "..." } or maven { url "..." } or maven { url '...' }
        for (i, _) in block.match_indices("maven {") {
            let sub = &block[i..];
            let brace_pos = sub.find('{').unwrap_or(sub.len());
            let inner = &sub[brace_pos + 1..];

            // Find the closing brace of this maven block
            let close_brace = match find_brace_block_end(inner) {
                Some(c) => c,
                None => continue,
            };
            let body = &inner[..close_brace];

            // Try url = "..." or url("...") or url '...' or url "..."
            let url = if let Some(url_pos) = body.find("url") {
                let after_url = body[url_pos + 3..].trim();
                if let Some(eq_pos) = after_url.find('=') {
                    let value = after_url[eq_pos + 1..].trim();
                    extract_string_literal(value)
                } else if let Some(rest) = after_url.strip_prefix('(') {
                    if let Some(_close) = find_matching_paren(rest) {
                        extract_string_literal(rest)
                    } else {
                        None
                    }
                } else {
                    // Groovy: url 'https://...' or url "https://..."
                    extract_string_literal(after_url)
                }
            } else {
                None
            };

            found_repos.push((
                i,
                ParsedRepository {
                    name: url.unwrap_or_else(|| format!("maven-custom-{}", found_repos.len())),
                    repo_type: "maven".to_string(),
                },
            ));
        }

        // Sort by source position to maintain declaration order
        found_repos.sort_unstable_by_key(|(pos, _)| *pos);
        result
            .repositories
            .extend(found_repos.into_iter().map(|(_, repo)| repo));
    }
}

/// Find the position of the closing brace (accounting for nesting and strings).
/// Returns the index of `}` within `content`.
fn find_brace_block_end(content: &str) -> Option<usize> {
    let mut depth = 1;
    let mut in_string = false;
    let mut string_char = ' ';
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() && depth > 0 {
        let c = bytes[i];
        if in_string {
            if c == string_char as u8 && !is_escaped(content, i) {
                in_string = false;
            }
        } else if c == b'"' || c == b'\'' {
            in_string = true;
            string_char = c as char;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Parse tasks block in Kotlin DSL.
fn parse_tasks_block(content: &str, result: &mut BuildScriptParseResult) {
    // Kotlin DSL: tasks.register("foo") { dependsOn("bar") }
    for (i, _) in content.match_indices("tasks.register(") {
        let args_start = i + 15;
        if let Some(close) = content[args_start..].find(')') {
            let args = &content[args_start..args_start + close];
            if let Some(task_name) = extract_string_literal(args) {
                let mut task_config = ParsedTaskConfig {
                    task_name,
                    depends_on: Vec::new(),
                    should_run_after: Vec::new(),
                    enabled: true,
                    ..Default::default()
                };

                // Find the block after the register call
                let after_close = args_start + close + 1;
                let rest = &content[after_close..];
                if let Some(brace_start) = rest.find('{') {
                    if let Some(block) = find_brace_block(content, after_close + brace_start) {
                        // Parse dependsOn("...")
                        for (di, _) in block.match_indices("dependsOn(") {
                            let da = di + 10;
                            if let Some(dc) = block[da..].find(')') {
                                if let Some(dep) = extract_string_literal(&block[da..da + dc]) {
                                    task_config.depends_on.push(dep);
                                }
                            }
                        }
                        // Parse shouldRunAfter("...")
                        for (di, _) in block.match_indices("shouldRunAfter(") {
                            let da = di + 15;
                            if let Some(dc) = block[da..].find(')') {
                                if let Some(dep) = extract_string_literal(&block[da..da + dc]) {
                                    task_config.should_run_after.push(dep);
                                }
                            }
                        }
                        // Parse enabled = false
                        if block.contains("enabled = false") || block.contains("enabled=false") {
                            task_config.enabled = false;
                        }
                    }
                }

                result.task_configs.push(task_config);
            }
        }
    }
}

/// Parse Groovy-style task configurations.
fn parse_groovy_tasks(content: &str, result: &mut BuildScriptParseResult) {
    // Groovy: task foo { dependsOn bar }
    for (i, _) in content.match_indices("task ") {
        let after = &content[i + 5..];
        // Skip if this is "tasks {"
        if after.starts_with('{') || after.starts_with('.') {
            continue;
        }
        // Get task name (until whitespace, {, or ()
        let name_end = after
            .find(|c: char| c.is_whitespace() || c == '{' || c == '(')
            .unwrap_or(after.len());
        let task_name = after[..name_end].trim().to_string();
        if task_name.is_empty() || task_name.starts_with('{') {
            continue;
        }

        let mut task_config = ParsedTaskConfig {
            task_name,
            depends_on: Vec::new(),
            should_run_after: Vec::new(),
            enabled: true,
            ..Default::default()
        };

        // Find the task block
        let search_from = i + 5 + name_end;
        let rest = &content[search_from..];
        if let Some(brace_pos) = rest.find('{') {
            if let Some(block) = find_brace_block(content, search_from + brace_pos) {
                // dependsOn 'bar' or dependsOn "bar" or dependsOn bar
                for (di, _) in block.match_indices("dependsOn ") {
                    let da = di + 10; // "dependsOn " = 10 chars
                    let rest = block[da..].trim_start();
                    // Try to extract a quoted string literal first
                    if let Some(dep) = extract_string_literal(rest) {
                        task_config.depends_on.push(dep);
                    } else {
                        // Unquoted form: take until whitespace or newline
                        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
                        let dep = rest[..end].trim().to_string();
                        if !dep.is_empty() {
                            task_config.depends_on.push(dep);
                        }
                    }
                }
                // shouldRunAfter 'bar' or shouldRunAfter "bar" or shouldRunAfter bar
                for (di, _) in block.match_indices("shouldRunAfter ") {
                    let da = di + 14; // "shouldRunAfter " = 14 chars
                    let rest = block[da..].trim_start();
                    if let Some(dep) = extract_string_literal(rest) {
                        task_config.should_run_after.push(dep);
                    } else {
                        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
                        let dep = rest[..end].trim().to_string();
                        if !dep.is_empty() {
                            task_config.should_run_after.push(dep);
                        }
                    }
                }
                if block.contains("enabled = false") || block.contains("enabled false") {
                    task_config.enabled = false;
                }
            }
        }

        result.task_configs.push(task_config);
    }
}

/// Parse top-level assignments: group, version, sourceCompatibility, targetCompatibility.
fn parse_top_level_assignments(content: &str, result: &mut BuildScriptParseResult) {
    for line in content.lines() {
        let line = line.trim();

        // group = "com.example"
        if let Some(eq_pos) = line.find("=") {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            match key {
                "group" => {
                    result.group = extract_string_literal(value).or_else(|| {
                        if value.is_empty() {
                            None
                        } else {
                            Some(value.to_string())
                        }
                    });
                }
                "version" => {
                    result.version = extract_string_literal(value).or_else(|| {
                        if value.is_empty() {
                            None
                        } else {
                            Some(value.to_string())
                        }
                    });
                }
                // Java toolchain: java { sourceCompatibility = ... }
                _ => {}
            }
        }
    }

    // Parse java { sourceCompatibility = ... targetCompatibility = ... }
    if let Some((_pos, block)) = find_top_level_block(content, "java") {
        for line in block.lines() {
            let line = line.trim();
            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim();
                let value = line[eq_pos + 1..].trim();

                match key {
                    "sourceCompatibility" | "sourceCompatibilityVersion" => {
                        // Strip both single and double quotes
                        let cleaned = value
                            .trim_start_matches('\'')
                            .trim_end_matches('\'')
                            .trim_start_matches('"')
                            .trim_end_matches('"')
                            .to_string();
                        result.source_compatibility = Some(cleaned);
                    }
                    "targetCompatibility" | "targetCompatibilityVersion" => {
                        let cleaned = value
                            .trim_start_matches('\'')
                            .trim_end_matches('\'')
                            .trim_start_matches('"')
                            .trim_end_matches('"')
                            .to_string();
                        result.target_compatibility = Some(cleaned);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Parse subproject include statements.
fn parse_subproject_includes(content: &str, result: &mut BuildScriptParseResult) {
    // Kotlin DSL: include(":app", ":lib")
    for (i, _) in content.match_indices("include(") {
        let args_start = i + 8;
        if let Some(close) = content[args_start..].find(')') {
            let args = content[args_start..args_start + close].trim();
            // Parse comma-separated quoted strings
            for part in args.split(',') {
                let part = part.trim();
                if let Some(path) = extract_string_literal(part) {
                    result.subprojects.push(ParsedSubproject { path });
                }
            }
        }
    }

    // settings.gradle: include ':app', ':lib' or include ":app", ":lib"
    // Single pass handles both quote styles via extract_string_literal
    for (i, _) in content.match_indices("include ") {
        let args_start = i + 8; // skip past "include "
                                // Find end of the include statement (newline or end of content)
        let line_end = content[args_start..]
            .find('\n')
            .unwrap_or(content.len() - args_start);
        let line = &content[args_start..args_start + line_end];
        // Only process if this looks like an include with quoted args
        if !line.starts_with('\'') && !line.starts_with('"') && !line.starts_with('(') {
            continue;
        }
        // Parse comma-separated quoted strings
        for part in line.split(',') {
            let part = part.trim();
            if let Some(path) = extract_string_literal(part) {
                // Deduplicate: skip if already added by include() form
                if !result.subprojects.iter().any(|s| s.path == path) {
                    result.subprojects.push(ParsedSubproject { path });
                }
            }
        }
    }
}

/// Parse a build script file.
pub fn parse_build_script_file(path: &Path) -> std::io::Result<BuildScriptParseResult> {
    let content = std::fs::read_to_string(path)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("build.gradle");
    Ok(parse_build_script(&content, file_name))
}

/// Parse multiple build script files and merge results.
pub fn parse_build_script_files(
    paths: &[&Path],
) -> Vec<(std::path::PathBuf, BuildScriptParseResult)> {
    paths
        .iter()
        .filter_map(|p| {
            let result = parse_build_script_file(p).ok()?;
            Some((p.to_path_buf(), result))
        })
        .collect()
}

/// Check if a path looks like a Gradle build script.
pub fn is_build_script(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    name == "build.gradle"
        || name == "build.gradle.kts"
        || name == "settings.gradle"
        || name == "settings.gradle.kts"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_kotlin_dsl_by_extension() {
        assert_eq!(
            detect_script_type("build.gradle.kts", ""),
            ScriptType::KotlinDsl
        );
        assert_eq!(detect_script_type("build.gradle", ""), ScriptType::Groovy);
        assert_eq!(detect_script_type("foo.txt", ""), ScriptType::Unknown);
    }

    #[test]
    fn test_detect_groovy_by_extension() {
        assert_eq!(detect_script_type("build.gradle", ""), ScriptType::Groovy);
    }

    #[test]
    fn test_detect_kotlin_dsl_by_content() {
        let content = r#"plugins { id("java") }"#;
        assert_eq!(
            detect_script_type("build.gradle", content),
            ScriptType::KotlinDsl
        );
    }

    #[test]
    fn test_detect_groovy_by_content() {
        let content = r#"plugins { id "java" }"#;
        assert_eq!(
            detect_script_type("build.gradle", content),
            ScriptType::Groovy
        );
    }

    #[test]
    fn test_parse_kotlin_dsl_plugins() {
        let content = r#"
plugins {
    id("java")
    id("org.springframework.boot") version "3.2.0"
    id("io.spring.dependency-management") version "1.1.4" apply false
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.script_type, ScriptType::KotlinDsl);
        assert_eq!(result.plugins.len(), 3);
        assert_eq!(result.plugins[0].id, "java");
        assert!(result.plugins[0].apply);
        assert_eq!(result.plugins[1].id, "org.springframework.boot");
        assert!(result.plugins[1].apply);
        assert_eq!(result.plugins[2].id, "io.spring.dependency-management");
        assert!(!result.plugins[2].apply);
    }

    #[test]
    fn test_parse_kotlin_dsl_dependencies() {
        let content = r#"
dependencies {
    implementation("com.example:lib:1.0")
    testImplementation("junit:junit:4.13.2")
    api(project(":core"))
    runtimeOnly("com.h2database:h2")
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.dependencies.len(), 4);
        assert_eq!(result.dependencies[0].configuration, "implementation");
        assert_eq!(result.dependencies[0].notation, "com.example:lib:1.0");
        assert_eq!(result.dependencies[1].configuration, "testImplementation");
        assert_eq!(result.dependencies[2].configuration, "api");
        assert_eq!(result.dependencies[2].notation, "project(\":core\")");
        assert_eq!(result.dependencies[3].configuration, "runtimeOnly");
    }

    #[test]
    fn test_parse_groovy_plugins() {
        let content = r#"
plugins {
    id "java"
    id "org.springframework.boot" version "3.2.0"
}
apply plugin: "java"
"#;
        let result = parse_build_script(content, "build.gradle");
        assert_eq!(result.script_type, ScriptType::Groovy);
        assert_eq!(result.plugins.len(), 3);
        assert_eq!(result.plugins[0].id, "java");
        assert!(result.plugins[2].apply);
    }

    #[test]
    fn test_parse_groovy_dependencies() {
        let content = r#"
dependencies {
    implementation 'com.example:lib:1.0'
    testImplementation 'junit:junit:4.13.2'
}
"#;
        let result = parse_build_script(content, "build.gradle");
        assert_eq!(result.dependencies.len(), 2);
        assert_eq!(result.dependencies[0].configuration, "implementation");
        assert_eq!(result.dependencies[0].notation, "com.example:lib:1.0");
    }

    #[test]
    fn test_parse_repositories() {
        let content = r#"
repositories {
    mavenCentral()
    google()
    gradlePluginPortal()
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.repositories.len(), 3);
        assert_eq!(result.repositories[0].name, "mavenCentral");
        assert_eq!(result.repositories[1].name, "google");
    }

    #[test]
    fn test_parse_kotlin_dsl_tasks() {
        let content = r#"
tasks.register("integrationTest") {
    dependsOn("test")
    shouldRunAfter("build")
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.task_configs.len(), 1);
        assert_eq!(result.task_configs[0].task_name, "integrationTest");
        assert_eq!(result.task_configs[0].depends_on, vec!["test"]);
        assert_eq!(result.task_configs[0].should_run_after, vec!["build"]);
        assert!(result.task_configs[0].enabled);
    }

    #[test]
    fn test_parse_task_disabled() {
        let content = r#"
tasks.register("slowTask") {
    enabled = false
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.task_configs.len(), 1);
        assert!(!result.task_configs[0].enabled);
    }

    #[test]
    fn test_parse_group_and_version() {
        let content = r#"
group = "com.example"
version = "1.0.0"
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.group.as_deref(), Some("com.example"));
        assert_eq!(result.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn test_parse_java_compatibility() {
        let content = r#"
java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(
            result.source_compatibility.as_deref(),
            Some("JavaVersion.VERSION_17")
        );
        assert_eq!(
            result.target_compatibility.as_deref(),
            Some("JavaVersion.VERSION_17")
        );
    }

    #[test]
    fn test_parse_subprojects_kotlin() {
        let content = r#"
include(":app", ":lib", ":core")
"#;
        let result = parse_build_script(content, "settings.gradle.kts");
        assert_eq!(result.subprojects.len(), 3);
        assert_eq!(result.subprojects[0].path, ":app");
        assert_eq!(result.subprojects[1].path, ":lib");
        assert_eq!(result.subprojects[2].path, ":core");
    }

    #[test]
    fn test_parse_subprojects_groovy() {
        let content = "include ':app', ':lib'\n";
        let result = parse_build_script(content, "settings.gradle");
        assert_eq!(result.subprojects.len(), 2);
        assert_eq!(result.subprojects[0].path, ":app");
    }

    #[test]
    fn test_parse_empty_script() {
        let result = parse_build_script("", "build.gradle.kts");
        assert_eq!(result.script_type, ScriptType::KotlinDsl);
        assert!(result.plugins.is_empty());
        assert!(result.dependencies.is_empty());
    }

    #[test]
    fn test_parse_unknown_script_type() {
        let result = parse_build_script("some random content", "script.txt");
        assert_eq!(result.script_type, ScriptType::Unknown);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_remove_block_comments() {
        let content = "hello /* comment */ world";
        assert_eq!(remove_block_comments(content), "hello  world");
    }

    #[test]
    fn test_remove_multiline_block_comment() {
        let content = "a/* multi\nline\ncomment */b";
        assert_eq!(remove_block_comments(content), "a\n\nb");
    }

    #[test]
    fn test_remove_line_comments() {
        let content = "foo // comment\nbar";
        assert_eq!(remove_line_comments(content), "foo \nbar");
    }

    #[test]
    fn test_remove_line_comments_preserves_strings() {
        let content = r#"val x = "url // path" // comment"#;
        let result = remove_line_comments(content);
        assert!(result.contains("url // path"));
        assert!(!result.contains("// comment"));
    }

    #[test]
    fn test_extract_string_literal_double_quotes() {
        assert_eq!(
            extract_string_literal("\"hello\""),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_extract_string_literal_single_quotes() {
        assert_eq!(extract_string_literal("'hello'"), Some("hello".to_string()));
    }

    #[test]
    fn test_extract_string_literal_no_quotes() {
        assert_eq!(extract_string_literal("hello"), None);
    }

    #[test]
    fn test_find_brace_block() {
        let content = "abc { def { ghi } jkl }";
        assert_eq!(
            find_brace_block(content, 4),
            Some(" def { ghi } jkl ".to_string())
        );
    }

    #[test]
    fn test_find_brace_block_nested_strings() {
        let content = "abc { val x = \"}\" }";
        assert_eq!(
            find_brace_block(content, 4),
            Some(" val x = \"}\" ".to_string())
        );
    }

    #[test]
    fn test_find_top_level_block() {
        let content = "dependencies { implementation(\"foo\") }";
        let result = find_top_level_block(content, "dependencies");
        assert!(result.is_some());
        let (_pos, block) = result.unwrap();
        assert!(block.contains("implementation"));
    }

    #[test]
    fn test_is_build_script() {
        assert!(is_build_script(Path::new("build.gradle")));
        assert!(is_build_script(Path::new("build.gradle.kts")));
        assert!(is_build_script(Path::new("settings.gradle")));
        assert!(is_build_script(Path::new("settings.gradle.kts")));
        assert!(!is_build_script(Path::new("main.rs")));
        assert!(!is_build_script(Path::new("foo.gradle.bak")));
    }

    #[test]
    fn test_parse_complex_kotlin_dsl() {
        let content = r#"
plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "1.9.22"
    id("org.springframework.boot") version "3.2.0"
    id("io.spring.dependency-management") version "1.1.4" apply false
}

group = "com.example"
version = "0.0.1-SNAPSHOT"

java {
    sourceCompatibility = JavaVersion.VERSION_17
}

repositories {
    mavenCentral()
}

dependencies {
    implementation("org.springframework.boot:spring-boot-starter-web")
    implementation("org.jetbrains.kotlin:kotlin-reflect")
    implementation("com.example:core:1.0")
    testImplementation("org.springframework.boot:spring-boot-starter-test")
    annotationProcessor("org.projectlombok:lombok")
}

tasks.register("integrationTest") {
    dependsOn("test")
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");

        assert_eq!(result.script_type, ScriptType::KotlinDsl);
        assert_eq!(result.plugins.len(), 4);
        assert_eq!(result.group.as_deref(), Some("com.example"));
        assert_eq!(result.version.as_deref(), Some("0.0.1-SNAPSHOT"));
        assert!(result.source_compatibility.is_some());
        assert_eq!(result.repositories.len(), 1);
        assert_eq!(result.dependencies.len(), 5);
        assert_eq!(result.task_configs.len(), 1);
    }

    #[test]
    fn test_parse_real_groovy_build() {
        let content = r#"
plugins {
    id 'java'
    id 'org.springframework.boot' version '3.2.0'
}

group 'com.example'
version '1.0.0'

java {
    sourceCompatibility = '17'
}

repositories {
    mavenCentral()
}

dependencies {
    implementation 'org.springframework.boot:spring-boot-starter-web'
    testImplementation 'org.springframework.boot:spring-boot-starter-test'
}

task integrationTest(type: Test) {
    useJUnitPlatform()
    shouldRunAfter test
}
"#;
        let result = parse_build_script(content, "build.gradle");

        assert_eq!(result.script_type, ScriptType::Groovy);
        assert_eq!(result.plugins.len(), 2);
        assert_eq!(result.dependencies.len(), 2);
        assert_eq!(result.task_configs.len(), 1);
        assert_eq!(result.task_configs[0].task_name, "integrationTest");
        assert_eq!(result.task_configs[0].should_run_after, vec!["test"]);
    }

    #[test]
    fn test_custom_maven_repo() {
        let content = r#"
repositories {
    maven {
        url = "https://repo.spring.io/snapshot"
    }
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.repositories.len(), 1);
        assert_eq!(result.repositories[0].repo_type, "maven");
        assert!(result.repositories[0].name.contains("spring.io"));
    }

    #[test]
    fn test_nested_braces_in_dep_closure() {
        let content = r#"dependencies {
    implementation('com.example:lib:1.0') {
        exclude group: 'org.slf4j', module: 'slf4j-log4j12'
    }
}
"#;
        // Debug: check find_top_level_block
        match super::find_top_level_block(content, "dependencies") {
            Some((_pos, block)) => {
                eprintln!(
                    "Block found (len={}): {:?}",
                    block.len(),
                    &block[..std::cmp::min(block.len(), 80)]
                );
            }
            None => {
                eprintln!("find_top_level_block returned None!");
            }
        }
        let mut result = BuildScriptParseResult::default();
        parse_groovy_dependencies(content, &mut result);
        eprintln!("deps found: {}", result.dependencies.len());
        for d in &result.dependencies {
            eprintln!("  {} -> {}", d.configuration, d.notation);
        }
        assert_eq!(result.dependencies.len(), 1);
        assert_eq!(result.dependencies[0].configuration, "implementation");
        assert_eq!(result.dependencies[0].notation, "com.example:lib:1.0");
    }

    #[test]
    fn test_double_quote_dep_groovy() {
        let content = r#"dependencies {
    implementation "com.example:lib:1.0"
}
"#;
        let mut result = BuildScriptParseResult::default();
        parse_groovy_dependencies(content, &mut result);
        eprintln!("deps found: {}", result.dependencies.len());
        for d in &result.dependencies {
            eprintln!("  {} -> {}", d.configuration, d.notation);
        }
        assert_eq!(result.dependencies.len(), 1);
    }

    #[test]
    fn test_depends_on_quoted() {
        let content = r#"task foo {
    dependsOn 'test'
}
"#;
        let mut result = BuildScriptParseResult::default();
        parse_groovy_tasks(content, &mut result);
        eprintln!("tasks found: {}", result.task_configs.len());
        for t in &result.task_configs {
            eprintln!(
                "  {} -> depends_on={:?} enabled={}",
                t.task_name, t.depends_on, t.enabled
            );
        }
        assert_eq!(result.task_configs.len(), 1);
        assert_eq!(result.task_configs[0].depends_on, vec!["test"]);
    }

    #[test]
    fn test_version_catalog_ref_kotlin() {
        let content = r#"
dependencies {
    implementation(libs.commons.lang3)
    api(libs.versions.java.get())
    testImplementation(platform(libs.androidx.test))
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.catalog_refs.len(), 3);
        assert_eq!(result.catalog_refs[0].configuration, "implementation");
        assert_eq!(result.catalog_refs[0].alias, "libs.commons.lang3");
        assert_eq!(result.catalog_refs[1].alias, "libs.versions.java");
        assert_eq!(result.catalog_refs[2].alias, "libs.androidx.test");
    }

    #[test]
    fn test_version_catalog_ref_groovy() {
        let content = r#"
dependencies {
    implementation libs.commons.lang3
}
"#;
        let result = parse_build_script(content, "build.gradle");
        assert_eq!(result.catalog_refs.len(), 1);
        assert_eq!(result.catalog_refs[0].configuration, "implementation");
        assert_eq!(result.catalog_refs[0].alias, "libs.commons.lang3");
    }

    #[test]
    fn test_buildscript_classpath() {
        let content = r#"
buildscript {
    dependencies {
        classpath("com.example:plugin:1.0")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:1.9.22")
    }
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert_eq!(result.buildscript_deps.len(), 2);
        assert_eq!(
            result.buildscript_deps[0].notation,
            "com.example:plugin:1.0"
        );
        assert_eq!(
            result.buildscript_deps[1].notation,
            "org.jetbrains.kotlin:kotlin-gradle-plugin:1.9.22"
        );
    }

    #[test]
    fn test_buildscript_classpath_groovy() {
        let content = r#"
buildscript {
    dependencies {
        classpath 'com.example:plugin:1.0'
    }
}
"#;
        let result = parse_build_script(content, "build.gradle");
        assert_eq!(result.buildscript_deps.len(), 1);
    }

    #[test]
    fn test_no_catalog_refs_without_libs() {
        let content = r#"
dependencies {
    implementation("com.example:lib:1.0")
}
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert!(result.catalog_refs.is_empty());
    }

    #[test]
    fn test_no_buildscript_without_block() {
        let content = r#"
plugins { id("java") }
dependencies { implementation("com.example:lib:1.0") }
"#;
        let result = parse_build_script(content, "build.gradle.kts");
        assert!(result.buildscript_deps.is_empty());
    }

    #[test]
    fn test_plugin_management_kotlin() {
        let content = r#"
pluginManagement {
    repositories {
        gradlePluginPortal()
        maven { url = uri("https://repo.example.com/plugins") }
        mavenCentral()
    }
}
"#;
        let result = parse_build_script(content, "settings.gradle.kts");
        let pm = result.plugin_management.as_ref().unwrap();
        assert_eq!(pm.repositories.len(), 3);
        assert_eq!(pm.repositories[0].name, "gradlePluginPortal");
        assert_eq!(pm.repositories[1].name, "https://repo.example.com/plugins");
        assert_eq!(pm.repositories[1].repo_type, "maven");
        assert_eq!(pm.repositories[2].name, "mavenCentral");
    }

    #[test]
    fn test_plugin_management_groovy() {
        let content = r#"
pluginManagement {
    repositories {
        gradlePluginPortal()
        maven {
            url 'https://repo.example.com/plugins'
        }
    }
}
"#;
        let result = parse_build_script(content, "settings.gradle");
        let pm = result.plugin_management.as_ref().unwrap();
        assert_eq!(pm.repositories.len(), 2);
        assert_eq!(pm.repositories[0].name, "gradlePluginPortal");
        assert_eq!(pm.repositories[1].name, "https://repo.example.com/plugins");
    }

    #[test]
    fn test_dependency_resolution_management_kotlin() {
        let content = r#"
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        mavenCentral()
        google()
    }
}
"#;
        let result = parse_build_script(content, "settings.gradle.kts");
        let drm = result.dependency_resolution_management.as_ref().unwrap();
        assert_eq!(
            drm.repositories_mode.as_deref(),
            Some("FAIL_ON_PROJECT_REPOS")
        );
        assert_eq!(drm.repositories.len(), 2);
        assert_eq!(drm.repositories[0].name, "mavenCentral");
        assert_eq!(drm.repositories[1].name, "google");
    }

    #[test]
    fn test_dependency_resolution_management_groovy() {
        let content = r#"
dependencyResolutionManagement {
    repositoriesMode = RepositoriesMode.PREFER_SETTINGS
    repositories {
        mavenCentral()
    }
}
"#;
        let result = parse_build_script(content, "settings.gradle");
        let drm = result.dependency_resolution_management.as_ref().unwrap();
        assert_eq!(
            drm.repositories_mode.as_deref(),
            Some("PREFER_SETTINGS")
        );
        assert_eq!(drm.repositories.len(), 1);
    }

    #[test]
    fn test_no_plugin_management_without_block() {
        let content = r#"
plugins { id("java") }
"#;
        let result = parse_build_script(content, "settings.gradle.kts");
        assert!(result.plugin_management.is_none());
        assert!(result.dependency_resolution_management.is_none());
    }

    #[test]
    fn test_full_settings_kotlin() {
        let content = r#"
pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        mavenCentral()
    }
}

rootProject.name = "my-app"
include(":app")
include(":lib")
"#;
        let result = parse_build_script(content, "settings.gradle.kts");
        let pm = result.plugin_management.as_ref().unwrap();
        assert_eq!(pm.repositories.len(), 2);

        let drm = result.dependency_resolution_management.as_ref().unwrap();
        assert_eq!(drm.repositories_mode.as_deref(), Some("FAIL_ON_PROJECT_REPOS"));
        assert_eq!(drm.repositories.len(), 1);

        assert_eq!(result.subprojects.len(), 2);
        assert_eq!(result.subprojects[0].path, ":app");
        assert_eq!(result.subprojects[1].path, ":lib");
    }
}
