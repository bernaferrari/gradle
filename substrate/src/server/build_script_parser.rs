use std::path::Path;

/// A parsed dependency declaration from a build script.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDependency {
    /// The dependency configuration (e.g. "implementation", "api", "testImplementation").
    pub configuration: String,
    /// The dependency notation (e.g. "com.example:lib:1.0", "project(:other)").
    pub notation: String,
}

/// A parsed plugin application from a build script.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedPlugin {
    /// The plugin ID or fully-qualified class name.
    pub id: String,
    /// Whether `apply false` was used (deferred application).
    pub apply: bool,
}

/// A parsed task dependency declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTaskDependency {
    /// The task path.
    pub path: String,
    /// Whether the dependency is "shouldRunAfter" (soft) vs dependsOn (hard).
    pub soft: bool,
}

/// A parsed task configuration block.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTaskConfig {
    /// The task name.
    pub task_name: String,
    /// Dependencies declared on this task.
    pub depends_on: Vec<String>,
    /// shouldRunAfter dependencies.
    pub should_run_after: Vec<String>,
    /// Whether the task is enabled (default: true).
    pub enabled: bool,
}

/// A parsed repositories declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRepository {
    /// Repository name or URL.
    pub name: String,
    /// Repository type (maven, mavenCentral, gradlePluginPortal, etc.).
    pub repo_type: String,
}

/// A parsed subprojects declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSubproject {
    /// The subproject path (e.g. ":app", ":lib").
    pub path: String,
}

/// The result of parsing a build script.
#[derive(Debug, Clone, Default)]
pub struct BuildScriptParseResult {
    /// Applied plugins.
    pub plugins: Vec<ParsedPlugin>,
    /// Dependencies (project and external).
    pub dependencies: Vec<ParsedDependency>,
    /// Task configurations.
    pub task_configs: Vec<ParsedTaskConfig>,
    /// Repositories.
    pub repositories: Vec<ParsedRepository>,
    /// Subprojects (from settings.gradle or include statements).
    pub subprojects: Vec<ParsedSubproject>,
    /// Source compatibility (java, kotlin).
    pub source_compatibility: Option<String>,
    /// Target compatibility.
    pub target_compatibility: Option<String>,
    /// Group ID.
    pub group: Option<String>,
    /// Version.
    pub version: Option<String>,
    /// The build script type detected.
    pub script_type: ScriptType,
    /// Parse errors or warnings.
    pub warnings: Vec<String>,
}

/// Detected build script type.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ScriptType {
    #[default]
    Unknown,
    KotlinDsl,
    Groovy,
}

/// Parse a Gradle build script (Kotlin DSL or Groovy) and extract
/// plugins, dependencies, task configs, repositories, and subprojects.
pub fn parse_build_script(content: &str, file_name: &str) -> BuildScriptParseResult {
    let script_type = detect_script_type(file_name, content);

    match script_type {
        ScriptType::KotlinDsl => parse_kotlin_dsl(content),
        ScriptType::Groovy => parse_groovy(content),
        ScriptType::Unknown => {
            let mut result = BuildScriptParseResult::default();
            result.warnings.push("Unknown build script type".to_string());
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

    // Parse repositories block: repositories { mavenCentral() }
    parse_repositories_block(&content_clean, &mut result);

    // Parse task configurations: tasks.register("foo") { dependsOn("bar") }
    parse_tasks_block(&content_clean, &mut result);

    // Parse top-level assignments
    parse_top_level_assignments(&content_clean, &mut result);

    // Parse subproject includes
    parse_subproject_includes(&content_clean, &mut result);

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

    // Parse repositories { mavenCentral() }
    parse_repositories_block(&content_clean, &mut result);

    // Parse task configurations
    parse_groovy_tasks(&content_clean, &mut result);

    // Parse top-level assignments
    parse_top_level_assignments(&content_clean, &mut result);

    // Parse subproject includes
    parse_subproject_includes(&content_clean, &mut result);

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
                    result.plugins.push(ParsedPlugin { id, apply });
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
                    result.plugins.push(ParsedPlugin { id, apply: true });
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
                if abs_i > 0 && block.as_bytes().get(abs_i - 1).is_some_and(|c| c.is_ascii_alphabetic()) {
                    search_from = abs_i + 1;
                    continue;
                }
                let args_start = abs_i + pattern.len();
                if let Some(close) = find_matching_paren(&block[args_start..]) {
                    let args = &block[args_start..args_start + close];
                    if let Some(notation) = extract_string_literal(args) {
                        found_deps.push((abs_i, ParsedDependency {
                            configuration: kw.to_string(),
                            notation,
                        }));
                    } else if args.trim().starts_with("project(") {
                        found_deps.push((abs_i, ParsedDependency {
                            configuration: kw.to_string(),
                            notation: args.trim().to_string(),
                        }));
                    }
                }
                search_from = abs_i + 1;
            }
        }

        // Sort by source position to maintain declaration order
        found_deps.sort_by_key(|(pos, _)| *pos);
        result.dependencies.extend(found_deps.into_iter().map(|(_, dep)| dep));
    }

    // Parse standalone dependency declarations outside a block:
    // val implementation by platform.deps(...)
    for kw in &["implementation", "api", "compileOnly", "runtimeOnly"] {
        let pattern = format!("val {} by ", kw);
        if let Some(_i) = content.find(&pattern) {
            // This is a version catalog reference, skip detailed parsing
            result.warnings.push(format!(
                "Version catalog dependency '{}' not fully parsed",
                kw
            ));
        }
    }
}

/// Parse Groovy-style plugins block.
fn parse_groovy_plugins(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "plugins") {
        // Groovy: id "foo"
        for (i, _) in block.match_indices("id ") {
            let args_start = i + 3;
            if let Some(end) = block[args_start..].find('\n') {
                let args = block[args_start..args_start + end].trim();
                if let Some(id) = extract_string_literal(args) {
                    let apply = !block[i..].contains("apply false");
                    result.plugins.push(ParsedPlugin { id, apply });
                }
            }
        }
    }

    // Groovy standalone: apply plugin: "foo"
    for (i, _) in content.match_indices("apply plugin:") {
        let args_start = i + 14;
        let end = content[args_start..].find('\n').unwrap_or(content.len() - args_start);
        let args = content[args_start..args_start + end].trim();
        if let Some(id) = extract_string_literal(args) {
            result.plugins.push(ParsedPlugin { id, apply: true });
        }
    }

    // Groovy: apply plugin: 'foo'
    for (i, _) in content.match_indices("apply plugin: '") {
        let args_start = i + 15;
        if let Some(end) = content[args_start..].find('\'') {
            let id = content[args_start..args_start + end].to_string();
            result.plugins.push(ParsedPlugin { id, apply: true });
        }
    }
}

/// Parse Groovy-style dependencies block.
fn parse_groovy_dependencies(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "dependencies") {
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

        for kw in &config_keywords {
            // Groovy: implementation 'com.example:lib:1.0' or implementation("...")
            let single_quote = format!("{} '", kw);
            let double_quote = format!("{}(", kw);
            let mut search_from = 0;

            while search_from < block.len() {
                let found = if let Some(i) = block[search_from..].find(&single_quote) {
                    let abs_i = search_from + i + single_quote.len();
                    if let Some(end) = block[abs_i..].find('\'') {
                        let notation = block[abs_i..abs_i + end].to_string();
                        result.dependencies.push(ParsedDependency {
                            configuration: kw.to_string(),
                            notation,
                        });
                        search_from = abs_i + end + 1;
                        continue;
                    }
                    abs_i
                } else {
                    block.len()
                };

                if let Some(i) = block[found..].find(&double_quote) {
                    let abs_i = found + i + double_quote.len();
                    if let Some(end) = block[abs_i..].find(')') {
                        let args = &block[abs_i..abs_i + end];
                        if let Some(notation) = extract_string_literal(args) {
                            result.dependencies.push(ParsedDependency {
                                configuration: kw.to_string(),
                                notation,
                            });
                        }
                        search_from = abs_i + end + 1;
                        continue;
                    }
                }

                search_from = block.len();
            }
        }
    }
}

/// Parse repositories block (works for both Kotlin DSL and Groovy).
fn parse_repositories_block(content: &str, result: &mut BuildScriptParseResult) {
    if let Some((_pos, block)) = find_top_level_block(content, "repositories") {
        if block.contains("mavenCentral()") {
            result.repositories.push(ParsedRepository {
                name: "mavenCentral".to_string(),
                repo_type: "maven".to_string(),
            });
        }
        if block.contains("google()") {
            result.repositories.push(ParsedRepository {
                name: "google".to_string(),
                repo_type: "maven".to_string(),
            });
        }
        if block.contains("gradlePluginPortal()") {
            result.repositories.push(ParsedRepository {
                name: "gradlePluginPortal".to_string(),
                repo_type: "gradlePluginPortal".to_string(),
            });
        }
        if block.contains("mavenLocal()") {
            result.repositories.push(ParsedRepository {
                name: "mavenLocal".to_string(),
                repo_type: "maven-local".to_string(),
            });
        }

        // Custom maven repos: maven { url = "..." }
        for (i, _) in block.match_indices("maven {") {
            let sub = &block[i..];
            if let Some(url_start) = sub.find("url ") {
                let after_url = &sub[url_start + 4..];
                // Skip past "url = " to get to the value
                let value_start = if after_url.starts_with("= ") { 2 } else { 0 };
                let url_end = after_url[value_start..]
                    .find('\n')
                    .unwrap_or(after_url.len() - value_start);
                let url_line = after_url[value_start..value_start + url_end].trim();
                if let Some(url) = extract_string_literal(url_line) {
                    result.repositories.push(ParsedRepository {
                        name: url.clone(),
                        repo_type: "maven".to_string(),
                    });
                }
            }
        }
    }
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
        };

        // Find the task block
        let search_from = i + 5 + name_end;
        let rest = &content[search_from..];
        if let Some(brace_pos) = rest.find('{') {
            if let Some(block) = find_brace_block(content, search_from + brace_pos) {
                // dependsOn 'bar'
                for (di, _) in block.match_indices("dependsOn ") {
                    let da = di + 11;
                    let end = block[da..]
                        .find(|c: char| c.is_whitespace())
                        .unwrap_or(block.len() - da);
                    let dep = block[da..da + end].trim();
                    if let Some(dep) = extract_string_literal(dep) {
                        task_config.depends_on.push(dep);
                    } else if !dep.is_empty() {
                        task_config.depends_on.push(dep.to_string());
                    }
                }
                // shouldRunAfter 'bar' or shouldRunAfter test
                for (di, _) in block.match_indices("shouldRunAfter ") {
                    let da = di + 15;
                    let end = block[da..]
                        .find(|c: char| c.is_whitespace())
                        .unwrap_or(block.len() - da);
                    let dep = block[da..da + end].trim();
                    if let Some(dep) = extract_string_literal(dep) {
                        task_config.should_run_after.push(dep);
                    } else if !dep.is_empty() {
                        task_config.should_run_after.push(dep.to_string());
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
                        result.source_compatibility = Some(value.trim_start_matches('"').trim_end_matches('"').to_string());
                    }
                    "targetCompatibility" | "targetCompatibilityVersion" => {
                        result.target_compatibility = Some(value.trim_start_matches('"').trim_end_matches('"').to_string());
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

    // settings.gradle: include ':app', ':lib'
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
        // Parse comma-separated single-quoted strings
        for part in line.split(',') {
            let part = part.trim();
            if let Some(path) = extract_string_literal(part) {
                result.subprojects.push(ParsedSubproject { path });
            }
        }
    }

    for (i, _) in content.match_indices("include \"") {
        let args_start = i + 9;
        let line_end = content[args_start..]
            .find('\n')
            .unwrap_or(content.len() - args_start);
        let line = &content[args_start..args_start + line_end];
        for part in line.split(',') {
            let part = part.trim();
            if let Some(path) = extract_string_literal(part) {
                result.subprojects.push(ParsedSubproject { path });
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
pub fn parse_build_script_files(paths: &[&Path]) -> Vec<(std::path::PathBuf, BuildScriptParseResult)> {
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
    name == "build.gradle" || name == "build.gradle.kts" || name == "settings.gradle" || name == "settings.gradle.kts"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_kotlin_dsl_by_extension() {
        assert_eq!(detect_script_type("build.gradle.kts", ""), ScriptType::KotlinDsl);
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
        assert_eq!(detect_script_type("build.gradle", content), ScriptType::KotlinDsl);
    }

    #[test]
    fn test_detect_groovy_by_content() {
        let content = r#"plugins { id "java" }"#;
        assert_eq!(detect_script_type("build.gradle", content), ScriptType::Groovy);
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
        assert_eq!(result.source_compatibility.as_deref(), Some("JavaVersion.VERSION_17"));
        assert_eq!(result.target_compatibility.as_deref(), Some("JavaVersion.VERSION_17"));
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
        assert_eq!(extract_string_literal("\"hello\""), Some("hello".to_string()));
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
        assert_eq!(find_brace_block(content, 4), Some(" def { ghi } jkl ".to_string()));
    }

    #[test]
    fn test_find_brace_block_nested_strings() {
        let content = "abc { val x = \"}\" }";
        assert_eq!(find_brace_block(content, 4), Some(" val x = \"}\" ".to_string()));
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
}
