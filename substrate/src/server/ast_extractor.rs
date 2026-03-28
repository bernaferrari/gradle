//! AST-based build script element extractor.
//!
//! Walks the parsed Groovy/Kotlin AST and extracts structured IR types
//! (`BuildScriptParseResult`) from Gradle DSL constructs.

use crate::server::build_script_types::*;
use crate::server::groovy_parser::ast::*;

/// Known dependency configuration names.
const DEPENDENCY_CONFIGS: &[&str] = &[
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

/// Strip surrounding quotes from a string value.
fn strip_quotes(s: &str) -> String {
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract build script elements from a parsed AST.
pub fn extract_from_ast(script: &Script, dialect: ScriptType) -> BuildScriptParseResult {
    let mut extractor = AstExtractor::new(dialect);
    extractor.extract(script)
}

// ---------------------------------------------------------------------------
// Internal extractor
// ---------------------------------------------------------------------------

struct AstExtractor {
    result: BuildScriptParseResult,
    #[allow(dead_code)]
    dialect: ScriptType,
}

impl AstExtractor {
    fn new(dialect: ScriptType) -> Self {
        Self {
            result: BuildScriptParseResult {
                script_type: dialect,
                ..Default::default()
            },
            dialect,
        }
    }

    fn extract(&mut self, script: &Script) -> BuildScriptParseResult {
        for stmt in &script.statements {
            self.visit_stmt(stmt);
        }
        std::mem::take(&mut self.result)
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(expr_stmt) => self.visit_expr(&expr_stmt.expr),
            Stmt::VarDecl(_) | Stmt::Import(_) | Stmt::Block(_) => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::MethodCall(mc) => {
                self.handle_method_call(mc);
            }
            Expr::Assignment(assignment) => {
                self.handle_assignment(assignment);
            }
            _ => {}
        }
    }

    // ── Method call dispatch ─────────────────────────────────────────────

    fn handle_method_call(&mut self, mc: &MethodCall) {
        let no_receiver = mc.receiver.is_none();

        // Get closure from either trailing_closure or a positional Closure argument
        let closure = mc
            .trailing_closure
            .as_deref()
            .or_else(|| Self::closure_from_args(mc));

        // Top-level blocks: plugins { }, dependencies { }, etc.
        if no_receiver {
            if let Some(closure) = closure {
                match mc.name.as_str() {
                "plugins" => {
                    self.handle_plugins_block(closure);
                    return;
                }
                "dependencies" => {
                    self.handle_dependencies_block(closure);
                    return;
                }
                "repositories" => {
                    self.handle_repositories_block(closure);
                    return;
                }
                "buildscript" => {
                    self.handle_buildscript_block(closure);
                    return;
                }
                "pluginManagement" => {
                    self.handle_plugin_management_block(closure);
                    return;
                }
                "dependencyResolutionManagement" => {
                    self.handle_dep_resolution_mgmt_block(closure);
                    return;
                }
                "java" => {
                    self.handle_java_block(closure);
                    return;
                }
                _ => {}
            }
        } // closure
        } // no_receiver

        // tasks.register("foo") { ... }
        if mc.name == "register" {
            if let Some(receiver) = &mc.receiver {
                if let Expr::Identifier(id) = receiver.as_ref() {
                    if id.name == "tasks" {
                        if let Some(task) = self.try_extract_task_config(mc) {
                            self.result.task_configs.push(task);
                        }
                        return;
                    }
                }
            }
        }

        // task("foo") { ... } or task foo { ... }
        if no_receiver && mc.name == "task" {
            if let Some(task) = self.try_extract_task_config(mc) {
                self.result.task_configs.push(task);
            }
            return;
        }

        // include(":app", ":lib")
        if no_receiver && mc.name == "include" {
            if let Some(subs) = self.try_extract_include(mc) {
                self.result.subprojects.extend(subs);
            }
            return;
        }

        // apply plugin: "java" (Groovy) or apply(plugin = "java") (Kotlin)
        if no_receiver && mc.name == "apply" {
            self.try_extract_apply_plugin(mc);
        }
    }

    // ── Block handlers ───────────────────────────────────────────────────

    fn handle_plugins_block(&mut self, closure: &Closure) {
        let mut current_plugin: Option<ParsedPlugin> = None;

        for stmt in &closure.body {
            if let Stmt::Expr(expr_stmt) = stmt {
                let expr = &*expr_stmt.expr;
                if let Some(plugin) = self.try_extract_plugin(expr) {
                    if let Some(pending) = current_plugin.take() {
                        self.result.plugins.push(pending);
                    }
                    current_plugin = Some(plugin);
                    // Parser may have consumed subsequent id(...) calls as
                    // no-paren args of the first id call. Scan for them.
                    if let Expr::MethodCall(mc) = expr {
                        self.scan_nested_id_plugins(mc, &mut current_plugin);
                    }
                } else if let Some((key, value, extra_apply, nested_plugin)) =
                    self.try_extract_modifier(expr)
                {
                    // Kotlin DSL: separate `version "..."` and `apply false` statements
                    if let Some(ref mut plugin) = current_plugin {
                        match key.as_str() {
                            "version" => plugin.version = Some(value),
                            "apply" => plugin.apply = value != "false",
                            _ => {}
                        }
                        if let Some(apply_val) = extra_apply {
                            plugin.apply = apply_val;
                        }
                    }
                    // A nested id() means the next plugin was consumed as an arg
                    if let Some(pending) = current_plugin.take() {
                        self.result.plugins.push(pending);
                    }
                    if let Some(plugin) = nested_plugin {
                        current_plugin = Some(plugin);
                    }
                }
            }
        }

        if let Some(plugin) = current_plugin {
            self.result.plugins.push(plugin);
        }
    }

    /// Scan MethodCall args for nested `id(...)` calls consumed by the
    /// parser's no-paren argument greediness.
    fn scan_nested_id_plugins(
        &mut self,
        mc: &MethodCall,
        current: &mut Option<ParsedPlugin>,
    ) {
        for arg in mc.arguments.iter().skip(1) {
            if let Arg::Positional { expr } = arg {
                if let Expr::MethodCall(inner_mc) = expr.as_ref() {
                    if inner_mc.name == "id" && inner_mc.receiver.is_none() {
                        if let Some(pending) = current.take() {
                            self.result.plugins.push(pending);
                        }
                        if let Some(plugin) = self.try_extract_plugin(expr) {
                            *current = Some(plugin);
                            // Recurse: the nested id() might also have consumed
                            // further id() calls as its own args.
                            self.scan_nested_id_plugins(inner_mc, current);
                        }
                    }
                }
            }
        }
    }

    fn handle_dependencies_block(&mut self, closure: &Closure) {
        for stmt in &closure.body {
            if let Stmt::Expr(expr_stmt) = stmt {
                if let Expr::MethodCall(mc) = &*expr_stmt.expr {
                    if mc.receiver.is_none() && DEPENDENCY_CONFIGS.contains(&mc.name.as_str()) {
                        // Check for version catalog ref first
                        if let Some(Arg::Positional { expr }) = mc.arguments.first() {
                            if let Some(catalog_ref) =
                                self.try_extract_catalog_ref(&mc.name, expr)
                            {
                                self.result.catalog_refs.push(catalog_ref);
                                continue;
                            }
                        }
                        // Regular dependency
                        if let Some(dep) =
                            self.try_extract_dependency(&mc.name, mc)
                        {
                            self.result.dependencies.push(dep);
                        }

                        // Groovy no-paren greedy consumption: the parser may have
                        // consumed subsequent `config 'notation'` pairs as extra
                        // positional args. Scan for (Identifier, string-like) pairs.
                        self.scan_groovy_dep_args(mc);
                    }
                }
            }
        }
    }

    /// Scan MethodCall args for additional Groovy dependency pairs consumed by
    /// the parser's no-paren argument greediness.
    /// e.g. `implementation 'foo:bar:1.0' testImplementation 'baz:qux:2.0'`
    /// becomes one MethodCall with 4 positional args.
    fn scan_groovy_dep_args(&mut self, mc: &MethodCall) {
        let args = &mc.arguments;
        if args.len() <= 1 {
            return;
        }
        let mut i = 1;
        while i < args.len() {
            let config_name = match &args[i] {
                // Plain identifier: testImplementation (rare, but handle it)
                Arg::Positional { expr } => match expr.as_ref() {
                    Expr::Identifier(id) if DEPENDENCY_CONFIGS.contains(&id.name.as_str()) => {
                        Some(id.name.clone())
                    }
                    // No-paren method call consumed as arg: testImplementation 'junit:...'
                    Expr::MethodCall(inner_mc)
                        if inner_mc.receiver.is_none()
                            && DEPENDENCY_CONFIGS.contains(&inner_mc.name.as_str()) =>
                    {
                        // The notation is inside the inner MethodCall's args
                        if let Some(notation) = inner_mc
                            .arguments
                            .first()
                            .and_then(|a| self.arg_to_string(a))
                        {
                            self.result.dependencies.push(ParsedDependency {
                                configuration: inner_mc.name.clone(),
                                notation,
                                line: Some(inner_mc.span.line),
                            });
                            i += 1;
                            continue;
                        }
                        None
                    }
                    _ => None,
                },
                _ => None,
            };

            if let Some(config) = config_name {
                // Identifier case: next arg is the notation
                if matches!(&args[i], Arg::Positional { expr } if matches!(expr.as_ref(), Expr::Identifier(_)))
                    && i + 1 < args.len()
                {
                    if let Arg::Positional { expr: notation_expr } = &args[i + 1] {
                        if let Some(notation) = self.expr_to_string(notation_expr) {
                            self.result.dependencies.push(ParsedDependency {
                                configuration: config,
                                notation,
                                line: Some(mc.span.line),
                            });
                            i += 2;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
    }

    fn handle_repositories_block(&mut self, closure: &Closure) {
        for stmt in &closure.body {
            if let Stmt::Expr(expr_stmt) = stmt {
                if let Some(repo) = self.try_extract_repository(&expr_stmt.expr) {
                    self.result.repositories.push(repo);
                }
            }
        }
    }

    fn handle_buildscript_block(&mut self, closure: &Closure) {
        for stmt in &closure.body {
            if let Stmt::Expr(expr_stmt) = stmt {
                if let Expr::MethodCall(mc) = &*expr_stmt.expr {
                    if mc.name == "dependencies" && mc.receiver.is_none() {
                        if let Some(inner_closure) = Self::get_closure(mc) {
                            for inner_stmt in &inner_closure.body {
                                if let Stmt::Expr(inner_expr) = inner_stmt {
                                    if let Expr::MethodCall(mc) = &*inner_expr.expr {
                                        if mc.name == "classpath" && mc.receiver.is_none() {
                                            if let Some(notation) = mc
                                                .arguments
                                                .first()
                                                .and_then(|a| self.arg_to_string(a))
                                            {
                                                self.result.buildscript_deps.push(
                                                    ParsedBuildScriptDep { notation },
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_plugin_management_block(&mut self, closure: &Closure) {
        let mut mgmt = ParsedPluginManagement::default();
        for stmt in &closure.body {
            if let Stmt::Expr(expr_stmt) = stmt {
                if let Expr::MethodCall(mc) = &*expr_stmt.expr {
                    if mc.name == "repositories" && mc.receiver.is_none() {
                        if let Some(inner_closure) = Self::get_closure(mc) {
                            for inner_stmt in &inner_closure.body {
                                if let Stmt::Expr(inner_expr) = inner_stmt {
                                    if let Some(repo) =
                                        self.try_extract_repository(&inner_expr.expr)
                                    {
                                        mgmt.repositories.push(ParsedPluginRepository {
                                            name: repo.name,
                                            repo_type: repo.repo_type,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !mgmt.repositories.is_empty() {
            self.result.plugin_management = Some(mgmt);
        }
    }

    fn handle_dep_resolution_mgmt_block(&mut self, closure: &Closure) {
        let mut mgmt = ParsedDependencyResolutionManagement::default();
        for stmt in &closure.body {
            if let Stmt::Expr(expr_stmt) = stmt {
                // repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
                if let Expr::MethodCall(mc) = &*expr_stmt.expr {
                    if let Some(receiver) = &mc.receiver {
                        let is_repos_mode = match receiver.as_ref() {
                            Expr::PropertyAccess(pa) => pa.property == "repositoriesMode",
                            Expr::Identifier(id) => id.name == "repositoriesMode",
                            _ => false,
                        };
                        if is_repos_mode {
                            if let Some(mode) = mc
                                .arguments
                                .first()
                                .and_then(|a| self.arg_to_string(a))
                            {
                                mgmt.repositories_mode = Some(mode);
                            }
                        }
                    }
                    // repositories { ... }
                    if mc.name == "repositories" && mc.receiver.is_none() {
                        if let Some(inner_closure) = Self::get_closure(mc) {
                            for inner_stmt in &inner_closure.body {
                                if let Stmt::Expr(inner_expr) = inner_stmt {
                                    if let Some(repo) =
                                        self.try_extract_repository(&inner_expr.expr)
                                    {
                                        mgmt.repositories.push(repo);
                                    }
                                }
                            }
                        }
                    }
                }
                // repositoriesMode = RepositoriesMode.PREFER_SETTINGS (Groovy assignment form)
                if let Expr::Assignment(a) = &*expr_stmt.expr {
                    let is_repos_mode = match a.target.as_ref() {
                        Expr::PropertyAccess(pa) => pa.property == "repositoriesMode",
                        Expr::Identifier(id) => id.name == "repositoriesMode",
                        _ => false,
                    };
                    if is_repos_mode {
                        if let Some(mode) = self.expr_to_string(&a.value) {
                            mgmt.repositories_mode = Some(mode);
                        }
                    }
                }
            }
        }
        if mgmt.repositories_mode.is_some() || !mgmt.repositories.is_empty() {
            self.result.dependency_resolution_management = Some(mgmt);
        }
    }

    fn handle_java_block(&mut self, closure: &Closure) {
        for stmt in &closure.body {
            if let Stmt::Expr(expr_stmt) = stmt {
                if let Expr::Assignment(a) = &*expr_stmt.expr {
                    let prop_name = match a.target.as_ref() {
                        Expr::PropertyAccess(pa) => Some(pa.property.as_str()),
                        Expr::Identifier(id) => Some(id.name.as_str()),
                        _ => None,
                    };
                    if let Some(name) = prop_name {
                        if let Some(value) = self.expr_to_string(&a.value) {
                            match name {
                                "sourceCompatibility" => {
                                    self.result.source_compatibility = Some(value);
                                }
                                "targetCompatibility" => {
                                    self.result.target_compatibility = Some(value);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_assignment(&mut self, assignment: &Assignment) {
        if let Expr::Identifier(id) = assignment.target.as_ref() {
            if let Some(value) = self.expr_to_string(&assignment.value) {
                match id.name.as_str() {
                    "group" => self.result.group = Some(value),
                    "version" => self.result.version = Some(value),
                    _ => {}
                }
            }
        }
    }

    // ── Individual element extractors ────────────────────────────────────

    fn try_extract_plugin(&self, expr: &Expr) -> Option<ParsedPlugin> {
        let mc = match expr {
            Expr::MethodCall(mc) if mc.name == "id" => mc,
            _ => return None,
        };

        let line = Some(mc.span.line);

        if mc.arguments.is_empty() {
            return None;
        }

        // Kotlin DSL: id("java") — single string arg
        if mc.arguments.len() == 1 {
            if let Some(id) = self.arg_to_string(&mc.arguments[0]) {
                return Some(ParsedPlugin {
                    id,
                    apply: true,
                    version: None,
                    line,
                });
            }
        }

        // Groovy no-paren: id "org.springframework.boot" version "3.2.0" apply false
        // Also handles: id("org.springframework.boot") version "3.2.0" apply false
        // where version/apply are parsed as MethodCall no-paren args
        if let Some(first_string) = self.arg_to_string(&mc.arguments[0]) {
            let mut plugin = ParsedPlugin {
                id: first_string,
                apply: true,
                version: None,
                line,
            };

            let mut i = 1;
            while i < mc.arguments.len() {
                if let Arg::Positional { expr } = &mc.arguments[i] {
                    match expr.as_ref() {
                        Expr::Identifier(id) => {
                            match id.name.as_str() {
                                "version" => {
                                    if i + 1 < mc.arguments.len() {
                                        if let Some(ver) = self.arg_to_string(&mc.arguments[i + 1]) {
                                            plugin.version = Some(ver);
                                        }
                                        i += 2;
                                        continue;
                                    }
                                }
                                "apply" => {
                                    if i + 1 < mc.arguments.len() {
                                        if let Arg::Positional { expr } = &mc.arguments[i + 1] {
                                            if let Expr::Boolean(b) = expr.as_ref() {
                                                plugin.apply = b.value;
                                            }
                                        }
                                        i += 2;
                                        continue;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Expr::MethodCall(inner_mc) => {
                            match inner_mc.name.as_str() {
                                "version" => {
                                    if let Some(ver) = inner_mc.arguments.first().and_then(|a| self.arg_to_string(a)) {
                                        plugin.version = Some(ver);
                                    }
                                    // Check for apply(false) inside version call
                                    for inner_arg in inner_mc.arguments.iter().skip(1) {
                                        if let Arg::Positional { expr } = inner_arg {
                                            if let Expr::MethodCall(mc3) = expr.as_ref() {
                                                if mc3.name == "apply" {
                                                    if let Some(Expr::Boolean(b)) = mc3.arguments.first().and_then(|a| match a {
                                                        Arg::Positional { expr } => Some(expr.as_ref()),
                                                        _ => None,
                                                    }) {
                                                        plugin.apply = b.value;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                "apply" => {
                                    if let Some(Expr::Boolean(b)) = inner_mc.arguments.first().and_then(|a| match a {
                                        Arg::Positional { expr } => Some(expr.as_ref()),
                                        _ => None,
                                    }) {
                                        plugin.apply = b.value;
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                i += 1;
            }

            return Some(plugin);
        }

        None
    }

    fn try_extract_modifier(
        &self,
        expr: &Expr,
    ) -> Option<(String, String, Option<bool>, Option<ParsedPlugin>)> {
        let mc = match expr {
            Expr::MethodCall(mc) => mc,
            _ => return None,
        };

        let key = mc.name.clone();
        let value = mc.arguments.first().and_then(|a| self.arg_to_string(a))?;

        let mut extra_apply = None;
        let mut nested_plugin: Option<ParsedPlugin> = None;
        let mut found_nested_id = false;

        // Check subsequent args for `apply false` and nested `id(...)` calls
        for arg in mc.arguments.iter().skip(1) {
            if let Arg::Positional { expr } = arg {
                match expr.as_ref() {
                    Expr::MethodCall(inner_mc) if inner_mc.name == "apply" => {
                        if !found_nested_id {
                            extra_apply = inner_mc.arguments.first().and_then(|ia| {
                                if let Arg::Positional { expr } = ia {
                                    if let Expr::Boolean(b) = expr.as_ref() {
                                        return Some(b.value);
                                    }
                                }
                                None
                            });
                        } else if let Some(ref mut plugin) = nested_plugin {
                            // apply after nested id() applies to the nested plugin
                            if let Some(Expr::Boolean(b)) = inner_mc.arguments.first().and_then(|a| match a {
                                Arg::Positional { expr } => Some(expr.as_ref()),
                                _ => None,
                            }) {
                                plugin.apply = b.value;
                            }
                        }
                    }
                    Expr::MethodCall(inner_mc) if inner_mc.name == "version" => {
                        if found_nested_id {
                            // version after nested id() applies to the nested plugin
                            if let Some(ref mut plugin) = nested_plugin {
                                if let Some(ver) = inner_mc.arguments.first().and_then(|a| self.arg_to_string(a)) {
                                    plugin.version = Some(ver);
                                }
                                // Check for nested apply(false) inside this version call
                                for inner_arg in inner_mc.arguments.iter().skip(1) {
                                    if let Arg::Positional { expr } = inner_arg {
                                        if let Expr::MethodCall(mc3) = expr.as_ref() {
                                            if mc3.name == "apply" {
                                                if let Some(Expr::Boolean(b)) = mc3.arguments.first().and_then(|a| match a {
                                                    Arg::Positional { expr } => Some(expr.as_ref()),
                                                    _ => None,
                                                }) {
                                                    plugin.apply = b.value;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Expr::MethodCall(inner_mc) if inner_mc.name == "id" => {
                        found_nested_id = true;
                        let mut plugin = ParsedPlugin {
                            id: String::new(),
                            apply: true,
                            version: None,
                            line: Some(inner_mc.span.line),
                        };
                        if let Some(id) = inner_mc.arguments.first().and_then(|a| self.arg_to_string(a)) {
                            plugin.id = id;
                        }
                        nested_plugin = Some(plugin);
                    }
                    _ => {}
                }
            }
        }

        Some((key, value, extra_apply, nested_plugin))
    }

    fn try_extract_dependency(
        &self,
        config: &str,
        mc: &MethodCall,
    ) -> Option<ParsedDependency> {
        let notation = mc.arguments.first().and_then(|a| match a {
            Arg::Positional { expr } => {
                // MethodCall args (e.g. project(":core")) need quotes preserved
                if matches!(expr.as_ref(), Expr::MethodCall(_)) {
                    self.expr_to_notation(expr)
                } else {
                    self.expr_to_string(expr)
                }
            }
            _ => self.arg_to_string(a),
        })?;
        Some(ParsedDependency {
            configuration: config.to_string(),
            notation,
            line: Some(mc.span.line),
        })
    }

    fn try_extract_task_config(&self, mc: &MethodCall) -> Option<ParsedTaskConfig> {
        let line = Some(mc.span.line);
        let task_name = mc.arguments.first().and_then(|a| self.arg_to_string(a))?;

        let mut config = ParsedTaskConfig {
            task_name,
            depends_on: Vec::new(),
            should_run_after: Vec::new(),
            enabled: true,
            line,
        };

        if let Some(closure) = Self::get_closure(mc) {
            for stmt in &closure.body {
                if let Stmt::Expr(expr_stmt) = stmt {
                    self.extract_task_property(&expr_stmt.expr, &mut config);
                }
            }
        }

        Some(config)
    }

    fn extract_task_property(&self, expr: &Expr, config: &mut ParsedTaskConfig) {
        if let Expr::MethodCall(mc) = expr {
            match mc.name.as_str() {
                "dependsOn" | "depends_on" => {
                    if let Some(dep) = mc.arguments.first().and_then(|a| self.arg_to_string(a)) {
                        config.depends_on.push(dep);
                    }
                }
                "shouldRunAfter" | "should_run_after" => {
                    if let Some(dep) = mc.arguments.first().and_then(|a| self.arg_to_string(a)) {
                        config.should_run_after.push(dep);
                    }
                }
                "enabled" => {
                    if let Some(Expr::Boolean(b)) = mc.arguments.first().and_then(|a| match a {
                        Arg::Positional { expr } => Some(expr.as_ref()),
                        _ => None,
                    }) {
                        config.enabled = b.value;
                    }
                }
                _ => {}
            }
        }
        // Also handle assignment form: enabled = false
        if let Expr::Assignment(a) = expr {
            if let Expr::Identifier(id) = a.target.as_ref() {
                if let Expr::Boolean(b) = a.value.as_ref() {
                    if id.name == "enabled" {
                        config.enabled = b.value;
                    }
                }
            }
        }
    }

    fn try_extract_repository(&self, expr: &Expr) -> Option<ParsedRepository> {
        let mc = match expr {
            Expr::MethodCall(mc) => mc,
            _ => return None,
        };

        match mc.name.as_str() {
            "mavenCentral" => Some(ParsedRepository {
                name: "mavenCentral".to_string(),
                repo_type: "maven".to_string(),
            }),
            "google" => Some(ParsedRepository {
                name: "google".to_string(),
                repo_type: "maven".to_string(),
            }),
            "gradlePluginPortal" => Some(ParsedRepository {
                name: "gradlePluginPortal".to_string(),
                repo_type: "gradlePluginPortal".to_string(),
            }),
            "mavenLocal" => Some(ParsedRepository {
                name: "mavenLocal".to_string(),
                repo_type: "maven-local".to_string(),
            }),
            "maven" => {
                // maven { url = uri("...") } or maven { url "..." }
                if let Some(closure) = Self::get_closure(mc) {
                    for stmt in &closure.body {
                        if let Stmt::Expr(expr_stmt) = stmt {
                            if let Expr::Assignment(a) = &*expr_stmt.expr {
                                let is_url = match a.target.as_ref() {
                                    Expr::PropertyAccess(pa) => pa.property == "url",
                                    Expr::Identifier(id) => id.name == "url",
                                    _ => false,
                                };
                                if is_url {
                                    if let Some(url) = self.expr_to_string(&a.value) {
                                        return Some(ParsedRepository {
                                            name: url,
                                            repo_type: "maven".to_string(),
                                        });
                                    }
                                }
                            }
                            // Also handle no-paren: url "https://..."
                            if let Expr::MethodCall(url_mc) = &*expr_stmt.expr {
                                if url_mc.name == "url" && url_mc.receiver.is_none() {
                                    if let Some(url) = url_mc
                                        .arguments
                                        .first()
                                        .and_then(|a| self.arg_to_string(a))
                                    {
                                        return Some(ParsedRepository {
                                            name: url,
                                            repo_type: "maven".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn try_extract_include(&self, mc: &MethodCall) -> Option<Vec<ParsedSubproject>> {
        let mut subs = Vec::with_capacity(mc.arguments.len());
        for arg in &mc.arguments {
            if let Some(path) = self.arg_to_string(arg) {
                subs.push(ParsedSubproject { path });
            }
        }
        if subs.is_empty() {
            None
        } else {
            Some(subs)
        }
    }

    fn try_extract_apply_plugin(&mut self, mc: &MethodCall) {
        // Groovy: apply plugin: "java" — "plugin" is a named arg, "java" is the value
        for arg in &mc.arguments {
            if let Arg::Named(na) = arg {
                if na.name == "plugin" {
                    if let Some(id) = self.expr_to_string(&na.value) {
                        self.result.plugins.push(ParsedPlugin {
                            id,
                            apply: true,
                            version: None,
                            line: Some(mc.span.line),
                        });
                    }
                    return;
                }
            }
        }
        // Kotlin: apply(plugin = "java")
        for arg in &mc.arguments {
            if let Arg::Named(na) = arg {
                if na.name == "plugin" {
                    if let Some(id) = self.expr_to_string(&na.value) {
                        self.result.plugins.push(ParsedPlugin {
                            id,
                            apply: true,
                            version: None,
                            line: Some(mc.span.line),
                        });
                    }
                    return;
                }
            }
        }
    }

    fn try_extract_catalog_ref(
        &self,
        config: &str,
        expr: &Expr,
    ) -> Option<ParsedVersionCatalogRef> {
        // Unwrap platform(...) wrapper: platform(libs.foo.bar) → libs.foo.bar
        let inner = if let Expr::MethodCall(mc) = expr {
            if mc.name == "platform" && mc.receiver.is_none() {
                mc.arguments.first().and_then(|a| match a {
                    Arg::Positional { expr } => Some(expr.as_ref()),
                    _ => None,
                })
            } else {
                None
            }
        } else {
            None
        };

        let target = inner.unwrap_or(expr);
        let alias = self.extract_property_chain(target)?;
        if !alias.starts_with("libs.") {
            return None;
        }
        Some(ParsedVersionCatalogRef {
            configuration: config.to_string(),
            alias,
        })
    }

    fn extract_property_chain(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Identifier(id) => Some(id.name.clone()),
            Expr::PropertyAccess(pa) => {
                let base = self.extract_property_chain(&pa.object_expr)?;
                Some(format!("{}.{}", base, pa.property))
            }
            Expr::MethodCall(mc) if mc.arguments.is_empty() && mc.name == "get" => {
                // libs.versions.java.get() → strip .get()
                self.extract_property_chain(mc.receiver.as_ref()?)
            }
            _ => None,
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn expr_to_string(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::String(lit) => lit.plain_value().map(|s| strip_quotes(&s)),
            Expr::Boolean(b) => Some(b.value.to_string()),
            Expr::Number(n) => Some(n.raw.clone()),
            Expr::Identifier(id) => Some(id.name.clone()),
            // JavaVersion.VERSION_17, RepositoriesMode.FAIL_ON_PROJECT_REPOS
            Expr::PropertyAccess(pa) => {
                let base = self.expr_to_string(&pa.object_expr)?;
                Some(format!("{}.{}", base, pa.property))
            }
            // uri("https://...") — unwrap to just the URL for repository extraction
            Expr::MethodCall(mc) if mc.receiver.is_none() && mc.name == "uri" => {
                mc.arguments.first().and_then(|a| self.arg_to_string(a))
            }
            _ => None,
        }
    }

    /// Convert expression to notation string, preserving quotes for MethodCall args.
    /// Used for dependency notation where `project(":core")` must keep its quotes.
    fn expr_to_notation(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::String(lit) => lit.plain_value(), // Keep quotes
            Expr::Boolean(b) => Some(b.value.to_string()),
            Expr::Number(n) => Some(n.raw.clone()),
            // project(":core"), platform(libs.foo)
            Expr::MethodCall(mc) if mc.receiver.is_none() => {
                let args: Vec<String> = mc
                    .arguments
                    .iter()
                    .filter_map(|a| match a {
                        Arg::Positional { expr } => self.expr_to_notation(expr),
                        _ => None,
                    })
                    .collect();
                if args.len() == mc.arguments.len() && !args.is_empty() {
                    Some(format!("{}({})", mc.name, args.join(", ")))
                } else {
                    None
                }
            }
            _ => self.expr_to_string(expr),
        }
    }

    fn arg_to_string(&self, arg: &Arg) -> Option<String> {
        match arg {
            Arg::Positional { expr } => self.expr_to_string(expr),
            Arg::Named(na) => self.expr_to_string(&na.value),
        }
    }

    /// Extract a Closure from positional arguments (when not in trailing_closure).
    fn closure_from_args(mc: &MethodCall) -> Option<&Closure> {
        // Check last positional arg for a closure
        for arg in mc.arguments.iter().rev() {
            if let Arg::Positional { expr } = arg {
                if let Expr::Closure(closure) = expr.as_ref() {
                    return Some(closure);
                }
            }
        }
        None
    }

    /// Get closure from either trailing_closure or args, for task/register patterns.
    fn get_closure(mc: &MethodCall) -> Option<&Closure> {
        mc.trailing_closure
            .as_deref()
            .or_else(|| Self::closure_from_args(mc))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::groovy_parser::parse;

    fn extract(content: &str) -> BuildScriptParseResult {
        let result = parse(content);
        assert!(
            result.errors.is_empty(),
            "parse errors: {:?}",
            result.errors
        );
        extract_from_ast(&result.script, ScriptType::Groovy)
    }

    fn extract_kotlin(content: &str) -> BuildScriptParseResult {
        let result = parse(content);
        assert!(
            result.errors.is_empty(),
            "parse errors: {:?}",
            result.errors
        );
        extract_from_ast(&result.script, ScriptType::KotlinDsl)
    }

    #[test]
    fn test_empty_script() {
        let result = extract("");
        assert!(result.plugins.is_empty());
        assert!(result.dependencies.is_empty());
    }

    #[test]
    fn test_kotlin_plugin_simple() {
        let result = extract_kotlin(r#"plugins { id("java") }"#);
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].id, "java");
        assert!(result.plugins[0].apply);
        assert_eq!(result.plugins[0].line, Some(1));
    }

    #[test]
    fn test_kotlin_plugin_with_version() {
        let result = extract_kotlin(
            r#"plugins { id("org.springframework.boot") version "3.2.0" }"#,
        );
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].id, "org.springframework.boot");
        assert_eq!(result.plugins[0].version.as_deref(), Some("3.2.0"));
    }

    #[test]
    fn test_kotlin_plugin_apply_false() {
        let result = extract_kotlin(
            r#"plugins { id("io.spring.dependency-management") version "1.1.4" apply false }"#,
        );
        assert_eq!(result.plugins.len(), 1);
        assert!(!result.plugins[0].apply);
        assert_eq!(
            result.plugins[0].version.as_deref(),
            Some("1.1.4")
        );
    }

    #[test]
    fn test_kotlin_multiple_plugins() {
        let result = extract_kotlin(
            r#"plugins {
                id("java")
                id("org.springframework.boot") version "3.2.0"
                id("io.spring.dependency-management") version "1.1.4" apply false
            }"#,
        );
        assert_eq!(result.plugins.len(), 3);
        assert_eq!(result.plugins[0].id, "java");
        assert!(result.plugins[0].apply);
        assert_eq!(result.plugins[1].version.as_deref(), Some("3.2.0"));
        assert!(!result.plugins[2].apply);
    }

    #[test]
    fn test_groovy_plugin_simple() {
        let result = extract(r#"plugins { id "java" }"#);
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].id, "java");
    }

    #[test]
    fn test_groovy_plugin_with_version_and_apply() {
        let result = extract(
            r#"plugins { id "org.springframework.boot" version "3.2.0" apply false }"#,
        );
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].id, "org.springframework.boot");
        assert_eq!(result.plugins[0].version.as_deref(), Some("3.2.0"));
        assert!(!result.plugins[0].apply);
    }

    #[test]
    fn test_kotlin_dependencies() {
        let result = extract_kotlin(
            r#"dependencies { implementation("com.example:lib:1.0"); testImplementation("junit:junit:4.13") }"#,
        );
        assert_eq!(result.dependencies.len(), 2);
        assert_eq!(result.dependencies[0].configuration, "implementation");
        assert_eq!(result.dependencies[0].notation, "com.example:lib:1.0");
        assert_eq!(result.dependencies[1].configuration, "testImplementation");
    }

    #[test]
    fn test_groovy_dependencies() {
        let result = extract(
            r#"dependencies { implementation 'com.example:lib:1.0'; testImplementation 'junit:junit:4.13' }"#,
        );
        assert_eq!(result.dependencies.len(), 2);
        assert_eq!(result.dependencies[0].notation, "com.example:lib:1.0");
    }

    #[test]
    fn test_repositories() {
        let result = extract_kotlin(
            r#"repositories { mavenCentral(); google(); gradlePluginPortal() }"#,
        );
        assert_eq!(result.repositories.len(), 3);
        assert_eq!(result.repositories[0].repo_type, "maven");
        assert_eq!(result.repositories[1].repo_type, "maven");
        assert_eq!(result.repositories[2].repo_type, "gradlePluginPortal");
    }

    #[test]
    fn test_tasks_register() {
        let result = extract_kotlin(
            r#"tasks.register("integrationTest") { dependsOn("test") }"#,
        );
        assert_eq!(result.task_configs.len(), 1);
        assert_eq!(result.task_configs[0].task_name, "integrationTest");
        assert_eq!(result.task_configs[0].depends_on, vec!["test"]);
    }

    #[test]
    fn test_task_declaration() {
        let result = extract(r#"task("integrationTest") { dependsOn("test") }"#);
        assert_eq!(result.task_configs.len(), 1);
        assert_eq!(result.task_configs[0].task_name, "integrationTest");
    }

    #[test]
    fn test_top_level_assignment() {
        let result = extract(r#"group = "com.example"; version = "1.0.0""#);
        assert_eq!(result.group.as_deref(), Some("com.example"));
        assert_eq!(result.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn test_java_block() {
        let result = extract_kotlin(
            r#"java { sourceCompatibility = "17"; targetCompatibility = "17" }"#,
        );
        assert_eq!(result.source_compatibility.as_deref(), Some("17"));
        assert_eq!(result.target_compatibility.as_deref(), Some("17"));
    }

    #[test]
    fn test_include() {
        let result = extract_kotlin(r#"include(":app", ":lib")"#);
        assert_eq!(result.subprojects.len(), 2);
        assert_eq!(result.subprojects[0].path, ":app");
        assert_eq!(result.subprojects[1].path, ":lib");
    }

    #[test]
    fn test_buildscript_classpath() {
        let result = extract_kotlin(
            r#"buildscript { dependencies { classpath("com.example:plugin:1.0") } }"#,
        );
        assert_eq!(result.buildscript_deps.len(), 1);
        assert_eq!(result.buildscript_deps[0].notation, "com.example:plugin:1.0");
    }

    #[test]
    fn test_version_catalog_ref() {
        let result = extract_kotlin(
            r#"dependencies { implementation(libs.commons.lang3) }"#,
        );
        assert_eq!(result.catalog_refs.len(), 1);
        assert_eq!(result.catalog_refs[0].alias, "libs.commons.lang3");
        assert_eq!(result.catalog_refs[0].configuration, "implementation");
    }

    #[test]
    fn test_complex_kotlin_dsl() {
        let parsed = parse(
            r#"plugins { id("java") }; dependencies { implementation("com.example:lib:1.0") }; repositories { mavenCentral() }; group = "com.example"; version = "1.0""#,
        );
        assert!(parsed.errors.is_empty(), "parse errors: {:?}", parsed.errors);
        let result = extract_from_ast(&parsed.script, ScriptType::KotlinDsl);
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.dependencies.len(), 1);
        assert_eq!(result.repositories.len(), 1);
        assert_eq!(result.group.as_deref(), Some("com.example"));
        assert_eq!(result.version.as_deref(), Some("1.0"));
    }

    #[test]
    fn test_plugin_management() {
        let result = extract_kotlin(
            r#"pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }"#,
        );
        assert!(result.plugin_management.is_some());
        let mgmt = result.plugin_management.unwrap();
        assert_eq!(mgmt.repositories.len(), 2);
    }
}
