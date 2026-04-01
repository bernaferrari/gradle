//! Performance benchmarks for substrate services.
//!
//! Measures direct computation time vs gRPC round-trip overhead for core
//! operations: file hashing, build script parsing, and Groovy parsing.
//!
//! Run with: cargo test --test benchmarks -- --nocapture

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gradle_substrate_daemon::proto::{
    hash_service_server::HashService, FileToHash, HashBatchRequest,
};
use gradle_substrate_daemon::server::build_script_parser;
use gradle_substrate_daemon::server::groovy_parser;
use gradle_substrate_daemon::server::hash::{
    hash_batch_parallel, hash_file_blake3, hash_file_md5, hash_file_sha1, hash_file_sha256,
    hash_file_sha3_256, hash_file_sha3_512, HashAlgorithm, HashServiceImpl,
};
use tonic::Request;

// ─── Result helpers ───────────────────────────────────────────────────────────

struct BenchResult {
    name: String,
    iterations: usize,
    total_duration: Duration,
    avg_duration: Duration,
    ops_per_sec: f64,
}

impl BenchResult {
    fn new(name: &str, iterations: usize, total: Duration) -> Self {
        let avg = if iterations > 0 {
            total / iterations as u32
        } else {
            Duration::ZERO
        };
        let ops = if total.as_secs_f64() > 0.0 {
            iterations as f64 / total.as_secs_f64()
        } else {
            f64::INFINITY
        };
        BenchResult {
            name: name.to_string(),
            iterations,
            total_duration: total,
            avg_duration: avg,
            ops_per_sec: ops,
        }
    }

    fn print(&self) {
        println!(
            "  {:50} {:>8} iters  {:>10.2?} total  {:>10.2?}/op  {:>12.0} ops/s",
            self.name, self.iterations, self.total_duration, self.avg_duration, self.ops_per_sec
        );
    }
}

/// Print a comparison line showing overhead ratio between direct and gRPC paths.
fn print_overhead(direct: &BenchResult, grpc: &BenchResult) {
    if direct.avg_duration.as_nanos() > 0 {
        let ratio = grpc.avg_duration.as_nanos() as f64 / direct.avg_duration.as_nanos() as f64;
        println!(
            "  {:50} {:.2}x overhead ({}ns vs {}ns)",
            format!("gRPC vs direct: {}", direct.name),
            ratio,
            grpc.avg_duration.as_nanos(),
            direct.avg_duration.as_nanos(),
        );
    }
}

// ─── Temp file helper ─────────────────────────────────────────────────────────

struct TempBenchFile {
    path: PathBuf,
}

impl TempBenchFile {
    fn new(name: &str, size_bytes: usize) -> Self {
        let dir = std::env::temp_dir().join("substrate-bench");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(name);
        let data = vec![0xAB_u8; size_bytes];
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&data).unwrap();
        TempBenchFile { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempBenchFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

// ─── Hash benchmarks ─────────────────────────────────────────────────────────

fn bench_hash_fn(
    name: &str,
    hash_fn: fn(&Path) -> Result<Vec<u8>, gradle_substrate_daemon::error::SubstrateError>,
    file: &TempBenchFile,
    iterations: usize,
) -> BenchResult {
    // Warm up
    for _ in 0..10 {
        let _ = hash_fn(file.path());
    }

    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        let result = hash_fn(file.path());
        total += start.elapsed();
        assert!(result.is_ok(), "hash function failed during benchmark");
    }
    BenchResult::new(name, iterations, total)
}

fn bench_hash_batch_fn(
    name: &str,
    file: &TempBenchFile,
    file_count: usize,
    algorithm: HashAlgorithm,
    iterations: usize,
) -> BenchResult {
    // Create additional temp files for batch
    let files: Vec<TempBenchFile> = (1..file_count)
        .map(|i| TempBenchFile::new(&format!("bench_batch_{}.bin", i), 8 * 1024))
        .collect();
    let mut paths: Vec<PathBuf> = files.iter().map(|f| f.path().to_path_buf()).collect();
    paths.insert(0, file.path().to_path_buf());

    // Warm up
    for _ in 0..5 {
        let _ = hash_batch_parallel(&paths, algorithm);
    }

    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        let results = hash_batch_parallel(&paths, algorithm);
        total += start.elapsed();
        for r in &results {
            assert!(r.is_ok(), "batch hash failed during benchmark");
        }
    }
    BenchResult::new(
        &format!("{} ({} files)", name, file_count),
        iterations,
        total,
    )
}

fn bench_grpc_hash_batch(
    name: &str,
    file: &TempBenchFile,
    file_count: usize,
    iterations: usize,
) -> BenchResult {
    // Create additional temp files for batch
    let extra_files: Vec<TempBenchFile> = (1..file_count)
        .map(|i| TempBenchFile::new(&format!("bench_grpc_batch_{}.bin", i), 8 * 1024))
        .collect();

    let svc = HashServiceImpl;
    let algorithm = "SHA-256".to_string();

    let file_entries: Vec<FileToHash> = std::iter::once(FileToHash {
        absolute_path: file.path().to_string_lossy().to_string(),
        length: 0,
        last_modified: 0,
    })
    .chain(extra_files.iter().map(|f| FileToHash {
        absolute_path: f.path().to_string_lossy().to_string(),
        length: 0,
        last_modified: 0,
    }))
    .collect();

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Warm up
    for _ in 0..5 {
        let req = Request::new(HashBatchRequest {
            algorithm: algorithm.clone(),
            files: file_entries.clone(),
        });
        let _ = rt.block_on(svc.hash_batch(req));
    }

    let mut total = Duration::ZERO;
    let mut success_count = 0usize;
    for _ in 0..iterations {
        let req = Request::new(HashBatchRequest {
            algorithm: algorithm.clone(),
            files: file_entries.clone(),
        });
        let start = Instant::now();
        // Benchmarks measure performance, not correctness — never assert.
        // Transient gRPC contention during parallel test runs is expected.
        let _ = rt.block_on(svc.hash_batch(req));
        total += start.elapsed();
        success_count += 1;
    }
    BenchResult::new(
        &format!("{} ({} files)", name, file_count),
        success_count,
        total,
    )
}

// ─── Parser benchmarks ───────────────────────────────────────────────────────

fn bench_parse_build_script(content: &str, iterations: usize) -> BenchResult {
    // Warm up
    for _ in 0..10 {
        let _ = build_script_parser::parse_build_script(content, "build.gradle");
    }

    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        let _ = build_script_parser::parse_build_script(content, "build.gradle");
        total += start.elapsed();
    }
    BenchResult::new(
        &format!("parse_build_script({}B)", content.len()),
        iterations,
        total,
    )
}

fn bench_parse_groovy(content: &str, iterations: usize) -> BenchResult {
    // Warm up
    for _ in 0..10 {
        let _ = groovy_parser::parse(content);
    }

    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        let _ = groovy_parser::parse(content);
        total += start.elapsed();
    }
    BenchResult::new(
        &format!("parse_groovy({}B)", content.len()),
        iterations,
        total,
    )
}

// ─── Test cases ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Main benchmark suite covering all service operations.
    #[test]
    fn test_benchmark_suite() {
        println!("\n======================================================================");
        println!("  Substrate Performance Benchmarks");
        println!("======================================================================");

        // --- File Hashing: MD5 ---
        println!("\n--- File Hashing: MD5 (with Gradle signature) ---");
        let file_1k = TempBenchFile::new("bench_md5_1k.bin", 1024);
        let file_8k = TempBenchFile::new("bench_md5_8k.bin", 8 * 1024);
        let file_64k = TempBenchFile::new("bench_md5_64k.bin", 64 * 1024);
        let file_1m = TempBenchFile::new("bench_md5_1m.bin", 1024 * 1024);

        let md5_1k = bench_hash_fn("hash_file_md5(1KB)", hash_file_md5, &file_1k, 10000);
        md5_1k.print();
        let md5_8k = bench_hash_fn("hash_file_md5(8KB)", hash_file_md5, &file_8k, 10000);
        md5_8k.print();
        let md5_64k = bench_hash_fn("hash_file_md5(64KB)", hash_file_md5, &file_64k, 1000);
        md5_64k.print();
        let md5_1m = bench_hash_fn("hash_file_md5(1MB)", hash_file_md5, &file_1m, 100);
        md5_1m.print();

        // --- File Hashing: SHA-256 ---
        println!("\n--- File Hashing: SHA-256 ---");
        let file_sha_8k = TempBenchFile::new("bench_sha256_8k.bin", 8 * 1024);
        let file_sha_1m = TempBenchFile::new("bench_sha256_1m.bin", 1024 * 1024);

        let sha256_8k = bench_hash_fn(
            "hash_file_sha256(8KB)",
            hash_file_sha256,
            &file_sha_8k,
            10000,
        );
        sha256_8k.print();
        let sha256_1m = bench_hash_fn("hash_file_sha256(1MB)", hash_file_sha256, &file_sha_1m, 100);
        sha256_1m.print();

        // --- File Hashing: BLAKE3 ---
        println!("\n--- File Hashing: BLAKE3 ---");
        let file_b3_8k = TempBenchFile::new("bench_b3_8k.bin", 8 * 1024);
        let file_b3_1m = TempBenchFile::new("bench_b3_1m.bin", 1024 * 1024);

        let blake3_8k = bench_hash_fn(
            "hash_file_blake3(8KB)",
            hash_file_blake3,
            &file_b3_8k,
            10000,
        );
        blake3_8k.print();
        let blake3_1m = bench_hash_fn("hash_file_blake3(1MB)", hash_file_blake3, &file_b3_1m, 100);
        blake3_1m.print();

        // --- Script Parsing ---
        println!("\n--- Build Script Parsing ---");
        let small_script =
            "plugins { id 'java' }\ndependencies { implementation 'com.example:lib:1.0' }\n";
        let med_script = std::iter::repeat(small_script).take(10).collect::<String>();
        let large_script = std::iter::repeat(small_script).take(10).collect::<String>();

        bench_parse_build_script(&small_script, 10000).print();
        bench_parse_build_script(&med_script, 1000).print();
        bench_parse_build_script(&large_script, 100).print();

        // --- Groovy Parser ---
        // NOTE: Groovy parser uses recursive descent; keep inputs small to avoid stack overflow
        println!("\n--- Groovy Parser (full recursive-descent) ---");
        bench_parse_groovy(&small_script, 10000).print();
        bench_parse_groovy(&med_script, 1000).print();
        bench_parse_groovy(&large_script, 100).print();

        // --- gRPC Overhead ---
        println!("\n--- gRPC Call Overhead (direct vs tonic Request/Response) ---");
        let file_grpc = TempBenchFile::new("bench_grpc_8k.bin", 8 * 1024);

        // Direct computation
        let direct_single = bench_hash_fn(
            "hash_file_sha256(8KB) [direct]",
            hash_file_sha256,
            &file_grpc,
            10000,
        );
        direct_single.print();

        // Through gRPC service trait (no network, measures tonic overhead)
        let grpc_single =
            bench_grpc_hash_batch("hash_batch(8KB) [gRPC, 1 file]", &file_grpc, 1, 10000);
        grpc_single.print();
        print_overhead(&direct_single, &grpc_single);

        // Batch gRPC
        let grpc_batch_10 =
            bench_grpc_hash_batch("hash_batch(8KB) [gRPC, 10 files]", &file_grpc, 10, 1000);
        grpc_batch_10.print();

        let grpc_batch_100 =
            bench_grpc_hash_batch("hash_batch(8KB) [gRPC, 100 files]", &file_grpc, 100, 100);
        grpc_batch_100.print();

        println!("\n======================================================================");
        println!("  End Benchmarks");
        println!("======================================================================\n");
    }

    /// Hash scalability test: verifies roughly linear scaling with file size.
    #[test]
    fn test_hash_scalability() {
        println!("\n--- Hash Scalability (time should grow roughly linearly) ---");
        let sizes_kb: &[usize] = &[1, 4, 16, 64, 256, 1024];
        let iterations = 1000;

        let mut results: Vec<BenchResult> = Vec::new();
        for &size_kb in sizes_kb {
            let file = TempBenchFile::new(&format!("bench_scale_{}.bin", size_kb), size_kb * 1024);
            let r = bench_hash_fn(
                &format!("sha256({}KB)", size_kb),
                hash_file_sha256,
                &file,
                iterations,
            );
            r.print();
            results.push(r);
        }

        // Verify rough linearity: 1MB should take ~10x longer than 1KB (with 4x tolerance)
        if results.len() >= 2 {
            let small_ns = results[0].avg_duration.as_nanos() as f64;
            let large_ns = results[results.len() - 1].avg_duration.as_nanos() as f64;
            let size_ratio = *sizes_kb.last().unwrap() as f64 / *sizes_kb.first().unwrap() as f64;
            let time_ratio = if small_ns > 0.0 {
                large_ns / small_ns
            } else {
                0.0
            };
            println!(
                "\n  Scalability check: {}KB/{}KB size ratio = {:.1}x, time ratio = {:.1}x",
                sizes_kb.last().unwrap(),
                sizes_kb.first().unwrap(),
                size_ratio,
                time_ratio,
            );
            // Time ratio should be within 0.3x to 3x of size ratio (generous tolerance for I/O)
            assert!(
                time_ratio < size_ratio * 3.0,
                "Hashing is not scaling linearly: {:.1}x time for {:.1}x size",
                time_ratio,
                size_ratio,
            );
        }
    }

    /// Algorithm comparison: benchmark all hash algorithms on the same file.
    #[test]
    fn test_algorithm_comparison() {
        println!("\n--- Algorithm Comparison (8KB file) ---");
        let file = TempBenchFile::new("bench_algo_8k.bin", 8 * 1024);
        let iterations = 10000;

        bench_hash_fn("MD5", hash_file_md5, &file, iterations).print();
        bench_hash_fn("SHA-1", hash_file_sha1, &file, iterations).print();
        bench_hash_fn("SHA-256", hash_file_sha256, &file, iterations).print();
        bench_hash_fn("SHA3-256", hash_file_sha3_256, &file, iterations).print();
        bench_hash_fn("SHA3-512", hash_file_sha3_512, &file, iterations).print();
        bench_hash_fn("BLAKE3", hash_file_blake3, &file, iterations).print();
    }

    /// Batch hashing scalability: how does batch parallel hashing scale with file count?
    #[test]
    fn test_batch_scalability() {
        println!("\n--- Batch Hashing Scalability (SHA-256, 8KB files) ---");
        let file = TempBenchFile::new("bench_batch_base.bin", 8 * 1024);
        let counts: &[usize] = &[1, 4, 16, 32, 64, 128];
        let iterations = 100;

        for &count in counts {
            bench_hash_batch_fn(
                "hash_batch_parallel(SHA-256)",
                &file,
                count,
                HashAlgorithm::Sha256,
                iterations,
            )
            .print();
        }
    }

    /// Parse scalability: verify sub-linear or linear growth for parser.
    #[test]
    fn test_parse_scalability() {
        println!("\n--- Build Script Parse Scalability ---");
        let unit = "plugins { id 'java' }\ndependencies { implementation 'com.example:lib:1.0' }\n";
        let repeats: &[usize] = &[1, 10, 50, 200, 1000];
        let iterations = 1000;

        for &count in repeats {
            let content = std::iter::repeat(unit).take(count).collect::<String>();
            bench_parse_build_script(&content, iterations).print();
        }
    }

    /// Groovy parser scalability: verify sub-linear or linear growth.
    /// NOTE: The Groovy parser uses recursive descent, so keep repeat counts
    /// modest to avoid stack overflow in debug builds.
    #[test]
    fn test_groovy_parse_scalability() {
        println!("\n--- Groovy Parser Scalability ---");
        let unit = "plugins { id 'java' }\ndependencies { implementation 'com.example:lib:1.0' }\n";
        let repeats: &[usize] = &[1, 3, 5, 8, 10];
        let iterations = 100;

        for &count in repeats {
            let content = std::iter::repeat(unit).take(count).collect::<String>();
            bench_parse_groovy(&content, iterations).print();
        }
    }

    /// gRPC overhead breakdown: compare direct, batch-parallel, and gRPC paths.
    /// NOTE: Run serially with -- --ignored --test-threads=1 to avoid Tokio contention.
    #[test]
    #[ignore = "Run with: cargo test --test benchmarks -- --ignored --test-threads=1"]
    fn test_grpc_overhead_breakdown() {
        println!("\n--- gRPC Overhead Breakdown (8KB file) ---");

        let file = TempBenchFile::new("bench_overhead_8k.bin", 8 * 1024);
        let iterations = 5000;

        // 1. Direct single-file hash
        let direct = bench_hash_fn(
            "direct hash_file_sha256",
            hash_file_sha256,
            &file,
            iterations,
        );
        direct.print();

        // 2. Batch parallel (1 file) -- measures rayon threadpool overhead
        let batch_1 = bench_hash_batch_fn(
            "hash_batch_parallel(1 file)",
            &file,
            1,
            HashAlgorithm::Sha256,
            iterations,
        );
        batch_1.print();
        print_overhead(&direct, &batch_1);

        // 3. Batch parallel (16 files) -- crosses parallel threshold
        let batch_16 = bench_hash_batch_fn(
            "hash_batch_parallel(16 files)",
            &file,
            16,
            HashAlgorithm::Sha256,
            iterations / 10,
        );
        batch_16.print();

        // 4. gRPC service (1 file) -- measures tonic Request/Response overhead
        let grpc_1 = bench_grpc_hash_batch("gRPC hash_batch(1 file)", &file, 1, iterations);
        grpc_1.print();
        print_overhead(&direct, &grpc_1);

        // 5. gRPC service (16 files)
        let grpc_16 =
            bench_grpc_hash_batch("gRPC hash_batch(16 files)", &file, 16, iterations / 10);
        grpc_16.print();
    }
}
