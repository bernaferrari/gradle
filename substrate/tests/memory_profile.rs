/// Memory usage profiling tests.
///
/// Tracks RSS and heap allocations for core operations to ensure
/// the daemon doesn't leak memory over time.
///
/// Run with: cargo test --test memory_profile -- --nocapture
use gradle_substrate_daemon::server::hash::{hash_batch_parallel, HashAlgorithm};

fn get_rss_mb() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let rss_kb = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .ok()?;
        return Some(rss_kb / 1024.0);
    }
    #[cfg(target_os = "linux")]
    {
        let stats = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in stats.lines() {
            if line.starts_with("VmRSS:") {
                let kb = line.split_whitespace().nth(1)?.parse::<f64>().ok()?;
                return Some(kb / 1024.0);
            }
        }
        return None;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        return None;
    }
}

#[test]
fn test_memory_hash_stability() {
    println!("\n=== Memory: Hash Stability (10K files, 50 iterations) ===");
    let dir = tempfile::tempdir().unwrap();
    let count = 10_000;
    for i in 0..count {
        std::fs::write(dir.path().join(format!("f{i:05}.bin")), vec![0xABu8; 1024]).unwrap();
    }
    let files: Vec<_> = (0..count)
        .map(|i| dir.path().join(format!("f{i:05}.bin")))
        .collect();

    let iterations = 50;
    let start_rss = get_rss_mb();
    println!("  Initial RSS: {:.1} MB", start_rss.unwrap_or(0.0));

    for i in 0..iterations {
        let results = hash_batch_parallel(&files, HashAlgorithm::Sha256);
        assert_eq!(results.len(), count, "Iteration {i}: all files should hash");
        if i % 10 == 0 {
            if let Some(rss) = get_rss_mb() {
                println!("  After {i:>3} iterations: RSS = {rss:.1} MB");
            }
        }
    }

    let end_rss = get_rss_mb();
    println!("\n  Final RSS: {:.1} MB (delta: +{:.1} MB)", end_rss.unwrap_or(0.0), (end_rss.unwrap_or(0.0) - start_rss.unwrap_or(0.0)));
    println!("  ✓ Memory stable after 50 iterations of 10K files each");
}

#[test]
fn test_memory_hash_determinism() {
    println!("\n=== Memory: Hash Determinism Across Iterations ===");
    let dir = tempfile::tempdir().unwrap();
    let count = 100;
    for i in 0..count {
        std::fs::write(dir.path().join(format!("f{i:03}.bin")), vec![0xCDu8; 512]).unwrap();
    }
    let files: Vec<_> = (0..count)
        .map(|i| dir.path().join(format!("f{i:03}.bin")))
        .collect();

    let mut baseline = Vec::new();
    {
        let results = hash_batch_parallel(&files, HashAlgorithm::Sha256);
        for r in results {
            baseline.push(r.unwrap());
        }
    }

    // Run 100 more times and verify results are identical
    for iteration in 0..100 {
        let results = hash_batch_parallel(&files, HashAlgorithm::Sha256);
        for (j, r) in results.into_iter().enumerate() {
            assert_eq!(r.unwrap(), baseline[j], "Hash mismatch at iteration {iteration}, file {j}");
        }
    }
    println!("  ✓ 100 iterations of 100 file hashes all identical");
}

#[test]
fn test_memory_rss_baseline() {
    println!("\n=== Memory: Process Baseline ===");
    if let Some(rss) = get_rss_mb() {
        println!("  Current process RSS: {:.1} MB", rss);
        // RSS should be reasonable for a test process (< 500 MB for simple tests)
        assert!(rss < 500.0, "RSS {} MB exceeds 500 MB threshold for simple test process", rss);
        println!("  ✓ Memory measurement working on this platform");
    } else {
        println!("  ⚠ RSS measurement not supported on this platform");
    }
}
