/// Fuzz-style parser stress testing: generates random/malformed input and verifies
/// the parser never panics, always returns a valid BuildScriptParseResult, and
/// never hangs (bounded by timeout).
use gradle_substrate_daemon::server::build_script_parser::parse_build_script;

/// Generate pseudo-random strings with a fixed seed for reproducibility.
struct FuzzRng {
    state: u64,
}

impl FuzzRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.state
    }
    fn next_char(&mut self) -> char {
        let printable = b"abcABC0123456789 \\'\"{}()[]<>;:=+-.*/|&~!@#$%^&*\n\t";
        let idx = (self.next() as usize) % printable.len();
        printable[idx] as char
    }
}

#[test]
fn fuzz_random_input_no_panic() {
    let seed = 42u64;
    let mut rng = FuzzRng::new(seed);
    for iteration in 0..1000 {
        let len = (iteration % 500) + 1;
        let mut input = String::with_capacity(len);
        for _ in 0..len {
            input.push(rng.next_char());
        }
        let _gradle_result = parse_build_script(&input, "build.gradle");
        let _kts_result = parse_build_script(&input, "build.gradle.kts");
    }
}

#[test]
fn fuzz_deep_nesting_no_panic() {
    for depth in 1..2000 {
        let input: String = (0..depth).map(|_| '{').collect::<String>()
            + "plugins { id 'java' }"
            + &(0..depth).map(|_| '}').collect::<String>();
        let result = parse_build_script(&input, "build.gradle");
        let _ = result.plugins.len();
    }
}

#[test]
fn fuzz_mismatched_quotes_no_panic() {
    let quotes = vec![
        "'java\"",
        "\"java'",
        "'java",
        "java'",
        "\"java",
        "java\"",
        "plugins { id '' }",
        "plugins { id \"\" }",
    ];
    for quote in &quotes {
        let input = format!("plugins {{ id {} }}", quote);
        let result = parse_build_script(&input, "build.gradle");
        let _ = result.plugins.len();
    }
}

#[test]
fn fuzz_unicode_no_panic() {
    let unicode_strings = vec![
        "日本語".to_string(),
        "café".to_string(),
        "plugins { id '日本語' }".to_string(),
        format!("plugins {{ id\t'java' }}"),
    ];
    for s in &unicode_strings {
        let result = parse_build_script(s, "build.gradle");
        let _ = result.plugins.len();
    }
}

#[test]
fn fuzz_extremely_long_lines_no_panic() {
    let long_dep = format!(
        "dependencies {{ implementation '{}' }}",
        (0..10000).map(|i| (b'a' + (i % 26) as u8) as char).collect::<String>()
    );
    let result = parse_build_script(&long_dep, "build.gradle");
    let _ = result.dependencies.len();
}

#[test]
fn fuzz_special_characters_no_panic() {
    let special_chars: Vec<String> = (0..128).map(|c| char::from_u32(c).unwrap().to_string()).collect();
    for ch in &special_chars {
        let input = format!("plugins {{ id 'java{}' }}", ch);
        let result = parse_build_script(&input, "build.gradle");
        let _ = result.plugins.len();
    }
}

#[test]
fn fuzz_empty_strings_no_panic() {
    let inputs = vec![
        "",
        "   ",
        "plugins {}",
        "plugins {  }",
        "dependencies {}",
        "repositories {}",
        "plugins {  id '' }",
    ];
    for input in &inputs {
        let result = parse_build_script(input, "build.gradle");
        let _ = result.plugins.len();
    }
}

#[test]
fn fuzz_stress_same_input() {
    let input = r#"
plugins {
    id 'java'
    id 'application' apply false
    id 'org.springframework.boot' version '3.0.0'
}
dependencies {
    implementation 'org.springframework.boot:spring-boot-starter-web'
    implementation 'org.springframework.boot:spring-boot-starter-data-jpa'
    testRuntimeOnly 'junit:junit:4.13.2'
    compileOnly 'org.projectlombok:lombok'
    annotationProcessor 'org.projectlombok:lombok'
}
repositories {
    mavenCentral()
    google()
    gradlePluginPortal()
}
"#;
    for _ in 0..1000 {
        let result = parse_build_script(input, "build.gradle");
        assert!(result.plugins.len() >= 1);
        assert!(result.dependencies.len() >= 1);
        assert!(result.repositories.len() >= 1);
    }

    let kotlin_input = r#"
plugins {
    id("java")
    id("com.example") version "1.0" apply false
}
dependencies {
    implementation("org.foo:bar:1.0")
    implementation(project(":core"))
}
"#;
    for _ in 0..1000 {
        let result = parse_build_script(kotlin_input, "build.gradle.kts");
        assert!(result.plugins.len() >= 1);
        assert!(result.dependencies.len() >= 1);
    }
}
