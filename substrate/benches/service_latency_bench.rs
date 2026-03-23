use criterion::{black_box, criterion_group, criterion_main, Criterion};

/// Benchmarks for key computational operations in the Rust substrate daemon.
/// These measure pure computation time, excluding gRPC and network overhead.
///
/// Compare against JVM equivalents:
///   - Hashing: MessageDigest.getInstance("MD5").digest() vs blake3::hash()
///   - POM parsing: MavenXpp3Reader vs byte-level scanner
///   - Property interpolation: String.replaceAll() vs iterative interpolation
///   - Version comparison: ComparableVersion.compareTo() vs compare_versions()
use gradle_substrate_daemon::server::dependency_resolution::{DependencyResolutionServiceImpl, compare_versions};

fn bench_pom_parsing(c: &mut Criterion) {
    let pom_kotlin = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>my-app</artifactId>
  <version>1.0.0</version>
  <properties>
    <spring.version>5.3.30</spring.version>
    <junit.version>4.13.2</junit.version>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-web</artifactId>
      <version>${spring.version}</version>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-data-jpa</artifactId>
      <version>${spring.version}</version>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-test</artifactId>
      <version>${spring.version}</version>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>${junit.version}</version>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>32.1.3-jre</version>
    </dependency>
    <dependency>
      <groupId>org.projectlombok</groupId>
      <artifactId>lombok</artifactId>
      <version>1.18.30</version>
      <scope>provided</scope>
    </dependency>
    <dependency>
      <groupId>org.mapstruct</groupId>
      <artifactId>mapstruct</artifactId>
      <version>1.5.5.Final</version>
    </dependency>
  </dependencies>
</project>"#;

    let pom_groovy = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>my-app</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-core</artifactId>
      <version>5.3.30</version>
    </dependency>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-context</artifactId>
      <version>5.3.30</version>
    </dependency>
    <dependency>
      <groupId>com.fasterxml.jackson.core</groupId>
      <artifactId>jackson-databind</artifactId>
      <version>2.15.3</version>
    </dependency>
  </dependencies>
</project>"#;

    let pom_large = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>large-project</artifactId>
  <version>1.0.0</version>
  <properties>
    <spring-boot.version>3.2.0</spring-boot.version>
    <spring-cloud.version>2023.0.0</spring-cloud.version>
    <jackson.version>2.15.3</jackson.version>
    <testcontainers.version>1.19.3</testcontainers.version>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-dependencies</artifactId>
        <version>${spring-boot.version}</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
      <dependency>
        <groupId>org.springframework.cloud</groupId>
        <artifactId>spring-cloud-dependencies</artifactId>
        <version>${spring-cloud.version}</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-web</artifactId>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-data-jpa</artifactId>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-security</artifactId>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-validation</artifactId>
    </dependency>
    <dependency>
      <groupId>org.springframework.cloud</groupId>
      <artifactId>spring-cloud-starter-config</artifactId>
    </dependency>
    <dependency>
      <groupId>org.springframework.cloud</groupId>
      <artifactId>spring-cloud-starter-netflix-eureka-client</artifactId>
    </dependency>
    <dependency>
      <groupId>org.springframework.cloud</groupId>
      <artifactId>spring-cloud-starter-openfeign</artifactId>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-test</artifactId>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>org.testcontainers</groupId>
      <artifactId>junit-jupiter</artifactId>
      <version>${testcontainers.version}</version>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>32.1.3-jre</version>
    </dependency>
    <dependency>
      <groupId>org.projectlombok</groupId>
      <artifactId>lombok</artifactId>
      <version>1.18.30</version>
      <scope>provided</scope>
    </dependency>
    <dependency>
      <groupId>org.mapstruct</groupId>
      <artifactId>mapstruct</artifactId>
      <version>1.5.5.Final</version>
    </dependency>
  </dependencies>
</project>"#;

    c.bench_function("parse_pom_kotlin_10_deps", |b| {
        b.iter(|| {
            let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(black_box(pom_kotlin));
            black_box(deps);
        });
    });

    c.bench_function("parse_pom_groovy_3_deps", |b| {
        b.iter(|| {
            let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(black_box(pom_groovy));
            black_box(deps);
        });
    });

    c.bench_function("parse_pom_large_16_deps", |b| {
        b.iter(|| {
            let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(black_box(pom_large));
            black_box(deps);
        });
    });
}

fn bench_pom_support(c: &mut Criterion) {
    let pom_kotlin = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>my-app</artifactId>
  <version>1.0.0</version>
  <properties>
    <spring.version>5.3.30</spring.version>
    <junit.version>4.13.2</junit.version>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-web</artifactId>
      <version>${spring.version}</version>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-data-jpa</artifactId>
      <version>${spring.version}</version>
    </dependency>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-test</artifactId>
      <version>${spring.version}</version>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>${junit.version}</version>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>32.1.3-jre</version>
    </dependency>
    <dependency>
      <groupId>org.projectlombok</groupId>
      <artifactId>lombok</artifactId>
      <version>1.18.30</version>
      <scope>provided</scope>
    </dependency>
    <dependency>
      <groupId>org.mapstruct</groupId>
      <artifactId>mapstruct</artifactId>
      <version>1.5.5.Final</version>
    </dependency>
  </dependencies>
</project>"#;

    c.bench_function("parse_properties_2_entries", |b| {
        b.iter(|| {
            let props = DependencyResolutionServiceImpl::parse_pom_properties(black_box(pom_kotlin));
            black_box(props);
        });
    });

    c.bench_function("parse_dependency_management", |b| {
        b.iter(|| {
            let managed = DependencyResolutionServiceImpl::parse_dependency_management(black_box(pom_kotlin));
            black_box(managed);
        });
    });
}

fn bench_interpolation(c: &mut Criterion) {
    let mut props = std::collections::HashMap::new();
    props.insert("spring.version".to_string(), "5.3.30".to_string());
    props.insert("junit.version".to_string(), "4.13.2".to_string());
    props.insert("jackson.version".to_string(), "2.15.3".to_string());
    let props = std::sync::Arc::new(props);

    c.bench_function("interpolate_single_property", |b| {
        let props = &props;
        b.iter(|| {
            let result = DependencyResolutionServiceImpl::interpolate_properties("${spring.version}", props);
            black_box(result);
        });
    });

    c.bench_function("interpolate_chain", |b| {
        let props = &props;
        b.iter(|| {
            let result = DependencyResolutionServiceImpl::interpolate_properties(
                "${spring.version}-jackson-${jackson.version}",
                props,
            );
            black_box(result);
        });
    });
}

fn bench_version_comparison(c: &mut Criterion) {
    c.bench_function("compare_simple_versions", |b| {
        b.iter(|| {
            let cmp = compare_versions("1.10.5", "1.9.0");
            black_box(cmp);
        });
    });

    c.bench_function("compare_pre_release", |b| {
        b.iter(|| {
            let cmp = compare_versions("1.0.0-beta", "1.0.0");
            black_box(cmp);
        });
    });

    c.bench_function("compare_with_snapshot", |b| {
        b.iter(|| {
            let cmp = compare_versions("1.0.0-SNAPSHOT", "1.0.0");
            black_box(cmp);
        });
    });
}

fn bench_conflict_resolution(c: &mut Criterion) {
    let mut deps = Vec::new();
    for i in 0..100u32 {
        deps.push(gradle_substrate_daemon::proto::ResolvedDependency {
            group: "com.example".to_string(),
            name: format!("lib-{}", i),
            version: format!("1.{}.0", i),
            selected_version: format!("1.{}.0", i),
            dependencies: Vec::new(),
            resolved: true,
            failure_reason: String::new(),
            artifact_url: String::new(),
            artifact_size: 0,
            artifact_sha256: String::new(),
        });
    }

    c.bench_function("resolve_conflicts_100_entries", |b| {
        b.iter(|| {
            let mut deps_copy = deps.clone();
            DependencyResolutionServiceImpl::resolve_conflicts(&mut deps_copy);
            black_box(deps_copy);
        });
    });
}

fn bench_hashing(c: &mut Criterion) {
    let small_data = vec![0x42u8; 1024]; // 1 KB
    let large_data = vec![0x42u8; 1024 * 1024]; // 1 MB

    c.bench_function("sha256_1kb", |b| {
        b.iter(|| {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(black_box(&small_data));
            black_box(format!("{:x}", hasher.finalize()));
        });
    });

    c.bench_function("sha256_1mb", |b| {
        b.iter(|| {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(black_box(&large_data));
            black_box(format!("{:x}", hasher.finalize()));
        });
    });

    c.bench_function("blake3_1kb", |b| {
        b.iter(|| {
            black_box(blake3::hash(&small_data));
        });
    });

    c.bench_function("blake3_1mb", |b| {
        b.iter(|| {
            black_box(blake3::hash(&large_data));
        });
    });

    c.bench_function("md5_1kb", |b| {
        b.iter(|| {
            use md5::Digest;
            let mut hasher = md5::Md5::new();
            hasher.update(black_box(&small_data));
            black_box(format!("{:x}", hasher.finalize()));
        });
    });

    c.bench_function("md5_1mb", |b| {
        b.iter(|| {
            use md5::Digest;
            let mut hasher = md5::Md5::new();
            hasher.update(black_box(&large_data));
            black_box(format!("{:x}", hasher.finalize()));
        });
    });
}

criterion_group!(
    benches,
    bench_pom_parsing,
    bench_pom_support,
    bench_interpolation,
    bench_version_comparison,
    bench_conflict_resolution,
    bench_hashing,
);
criterion_main!(benches);
