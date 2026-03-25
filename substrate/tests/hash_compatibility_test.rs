use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Cross-language hash compatibility test.
///
/// Writes a known file, computes its hash using Rust's implementation,
/// then computes the same hash using Java's DefaultStreamHasher via a small Java program,
/// and compares the results.

const TEST_CONTENT: &[u8] =
    b"Hello, Gradle Rust Substrate! This is a test file for cross-language hash verification.\n";

fn write_test_file(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("test_file.txt");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(TEST_CONTENT).unwrap();
    path
}

fn compute_rust_hash(path: &Path) -> Vec<u8> {
    gradle_substrate_daemon::server::hash::hash_file_md5(path).unwrap()
}

fn compute_java_hash(path: &Path) -> Vec<u8> {
    // This Java code exactly replicates DefaultStreamHasher.doHash():
    // 1. Compute SIGNATURE = Hashing.signature(DefaultStreamHasher.class)
    //    = Hashing.signature("CLASS:" + "org.gradle.internal.hash.DefaultStreamHasher")
    //    which uses DefaultHasher.putString for both strings
    // 2. New MD5 digest, putHash(signature), then putBytes(file content in 8192-byte chunks)
    let java_code = format!(
        r#"
import java.io.*;
import java.security.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;

public class ComputeHash {{
    public static void main(String[] args) throws Exception {{
        File file = new File(new File("{}").getCanonicalPath());
        MessageDigest digest = MessageDigest.getInstance("MD5");
        ByteBuffer buf = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN);

        // === Compute signature ===
        // Hashing.signature(Class) calls signature("CLASS:" + className)
        // signature(String thing) uses DefaultHasher:
        //   DefaultHasher.putString(s) = PrimitiveHasher.putInt(s.length()) + PrimitiveHasher.putString(s)
        //   PrimitiveHasher.putInt(v) = digest.update(int32_le(v))
        //   PrimitiveHasher.putString(s) = digest.update(s.getBytes(UTF_8))

        String sigLabel = "SIGNATURE";
        String className = "CLASS:" + "org.gradle.internal.hash.DefaultStreamHasher";

        // DefaultHasher.putString("SIGNATURE")
        buf.clear(); buf.putInt(sigLabel.length());
        digest.update(buf.array(), 0, 4);
        digest.update(sigLabel.getBytes(StandardCharsets.UTF_8));

        // DefaultHasher.putString("CLASS:org.gradle.internal.hash.DefaultStreamHasher")
        buf.clear(); buf.putInt(className.length());
        digest.update(buf.array(), 0, 4);
        digest.update(className.getBytes(StandardCharsets.UTF_8));

        byte[] signature = digest.digest();

        // === Compute file hash ===
        // DefaultStreamHasher.doHash: new PrimitiveHasher, putHash(SIGNATURE), putBytes(file chunks)
        digest = MessageDigest.getInstance("MD5");
        digest.update(signature);

        FileInputStream fis = new FileInputStream(file);
        byte[] buffer = new byte[8192];
        int n;
        while ((n = fis.read(buffer)) != -1) {{
            digest.update(buffer, 0, n);
        }}
        fis.close();

        byte[] hash = digest.digest();
        StringBuilder sb = new StringBuilder();
        for (byte b : hash) {{
            sb.append(String.format("%02x", b));
        }}
        System.out.print(sb.toString());
    }}
}}
"#,
        path.to_string_lossy()
    );

    let dir = tempfile::tempdir().unwrap();
    let java_file = dir.path().join("ComputeHash.java");
    fs::write(&java_file, java_code).unwrap();

    let output = Command::new("javac")
        .arg(&java_file)
        .current_dir(dir.path())
        .output()
        .expect("Failed to compile Java hash program");

    if !output.status.success() {
        panic!(
            "Java compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = Command::new("java")
        .arg("-cp")
        .arg(dir.path())
        .arg("ComputeHash")
        .output()
        .expect("Failed to run Java hash program");

    if !output.status.success() {
        panic!(
            "Java execution failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let hex = String::from_utf8_lossy(&output.stdout).trim().to_string();
    hex_to_bytes(&hex)
}

/// Compile and run a Java program that computes a raw SHA-256 hash of a file
/// (no Gradle signature wrapping, just straight MessageDigest SHA-256).
fn compute_java_sha256(path: &Path) -> Vec<u8> {
    let java_code = format!(
        r#"
import java.io.*;
import java.security.*;

public class ComputeSha256 {{
    public static void main(String[] args) throws Exception {{
        File file = new File(new File("{}").getCanonicalPath());
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        FileInputStream fis = new FileInputStream(file);
        byte[] buffer = new byte[8192];
        int n;
        while ((n = fis.read(buffer)) != -1) {{
            digest.update(buffer, 0, n);
        }}
        fis.close();
        byte[] hash = digest.digest();
        StringBuilder sb = new StringBuilder();
        for (byte b : hash) {{
            sb.append(String.format("%02x", b));
        }}
        System.out.print(sb.toString());
    }}
}}
"#,
        path.to_string_lossy()
    );

    let dir = tempfile::tempdir().unwrap();
    let java_file = dir.path().join("ComputeSha256.java");
    fs::write(&java_file, java_code).unwrap();

    let output = Command::new("javac")
        .arg(&java_file)
        .current_dir(dir.path())
        .output()
        .expect("Failed to compile Java SHA-256 program");

    if !output.status.success() {
        panic!(
            "Java compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = Command::new("java")
        .arg("-cp")
        .arg(dir.path())
        .arg("ComputeSha256")
        .output()
        .expect("Failed to run Java SHA-256 program");

    if !output.status.success() {
        panic!(
            "Java execution failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let hex = String::from_utf8_lossy(&output.stdout).trim().to_string();
    hex_to_bytes(&hex)
}

/// Compile and run a Java program that computes a raw SHA-1 hash of a file
/// (no Gradle signature wrapping, just straight MessageDigest SHA-1).
fn compute_java_sha1(path: &Path) -> Vec<u8> {
    let java_code = format!(
        r#"
import java.io.*;
import java.security.*;

public class ComputeSha1 {{
    public static void main(String[] args) throws Exception {{
        File file = new File(new File("{}").getCanonicalPath());
        MessageDigest digest = MessageDigest.getInstance("SHA-1");
        FileInputStream fis = new FileInputStream(file);
        byte[] buffer = new byte[8192];
        int n;
        while ((n = fis.read(buffer)) != -1) {{
            digest.update(buffer, 0, n);
        }}
        fis.close();
        byte[] hash = digest.digest();
        StringBuilder sb = new StringBuilder();
        for (byte b : hash) {{
            sb.append(String.format("%02x", b));
        }}
        System.out.print(sb.toString());
    }}
}}
"#,
        path.to_string_lossy()
    );

    let dir = tempfile::tempdir().unwrap();
    let java_file = dir.path().join("ComputeSha1.java");
    fs::write(&java_file, java_code).unwrap();

    let output = Command::new("javac")
        .arg(&java_file)
        .current_dir(dir.path())
        .output()
        .expect("Failed to compile Java SHA-1 program");

    if !output.status.success() {
        panic!(
            "Java compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = Command::new("java")
        .arg("-cp")
        .arg(dir.path())
        .arg("ComputeSha1")
        .output()
        .expect("Failed to run Java SHA-1 program");

    if !output.status.success() {
        panic!(
            "Java execution failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let hex = String::from_utf8_lossy(&output.stdout).trim().to_string();
    hex_to_bytes(&hex)
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn format_hash(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// =========================================================================
// Existing MD5 cross-language tests (unchanged)
// =========================================================================

#[test]
fn test_cross_language_hash_compatibility() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = write_test_file(dir.path());

    let rust_hash = compute_rust_hash(&test_file);
    let java_hash = compute_java_hash(&test_file);

    assert_eq!(rust_hash.len(), 16, "Rust hash should be 16 bytes (MD5)");
    assert_eq!(java_hash.len(), 16, "Java hash should be 16 bytes (MD5)");
    assert_eq!(
        rust_hash,
        java_hash,
        "Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_empty_file_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let empty_file = dir.path().join("empty.txt");
    fs::write(&empty_file, "").unwrap();

    let rust_hash = compute_rust_hash(&empty_file);
    let java_hash = compute_java_hash(&empty_file);

    assert_eq!(
        rust_hash,
        java_hash,
        "Empty file hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_large_file_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let large_file = dir.path().join("large.bin");
    let data = vec![0x42u8; 100_000];
    fs::write(&large_file, &data).unwrap();

    let rust_hash = compute_rust_hash(&large_file);
    let java_hash = compute_java_hash(&large_file);

    assert_eq!(
        rust_hash,
        java_hash,
        "Large file hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

// =========================================================================
// SHA-256 cross-language tests
// =========================================================================

#[test]
fn test_sha256_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = write_test_file(dir.path());

    let rust_hash = gradle_substrate_daemon::server::hash::hash_file_sha256(&test_file).unwrap();
    let java_hash = compute_java_sha256(&test_file);

    assert_eq!(rust_hash.len(), 32, "Rust SHA-256 hash should be 32 bytes");
    assert_eq!(java_hash.len(), 32, "Java SHA-256 hash should be 32 bytes");
    assert_eq!(
        rust_hash,
        java_hash,
        "SHA-256: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_sha256_cross_language_empty() {
    let dir = tempfile::tempdir().unwrap();
    let empty_file = dir.path().join("empty.txt");
    fs::write(&empty_file, "").unwrap();

    let rust_hash = gradle_substrate_daemon::server::hash::hash_file_sha256(&empty_file).unwrap();
    let java_hash = compute_java_sha256(&empty_file);

    assert_eq!(rust_hash.len(), 32);
    assert_eq!(java_hash.len(), 32);
    assert_eq!(
        rust_hash,
        java_hash,
        "SHA-256 empty file: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
    // SHA-256 of empty input is a well-known constant
    assert_eq!(
        format_hash(&rust_hash),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "SHA-256 of empty file should match the known digest"
    );
}

// =========================================================================
// SHA-1 cross-language tests
// =========================================================================

#[test]
fn test_sha1_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = write_test_file(dir.path());

    let rust_hash = gradle_substrate_daemon::server::hash::hash_file_sha1(&test_file).unwrap();
    let java_hash = compute_java_sha1(&test_file);

    assert_eq!(rust_hash.len(), 20, "Rust SHA-1 hash should be 20 bytes");
    assert_eq!(java_hash.len(), 20, "Java SHA-1 hash should be 20 bytes");
    assert_eq!(
        rust_hash,
        java_hash,
        "SHA-1: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_sha1_cross_language_empty() {
    let dir = tempfile::tempdir().unwrap();
    let empty_file = dir.path().join("empty.txt");
    fs::write(&empty_file, "").unwrap();

    let rust_hash = gradle_substrate_daemon::server::hash::hash_file_sha1(&empty_file).unwrap();
    let java_hash = compute_java_sha1(&empty_file);

    assert_eq!(rust_hash.len(), 20);
    assert_eq!(java_hash.len(), 20);
    assert_eq!(
        rust_hash,
        java_hash,
        "SHA-1 empty file: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
    // SHA-1 of empty input is a well-known constant
    assert_eq!(
        format_hash(&rust_hash),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709",
        "SHA-1 of empty file should match the known digest"
    );
}

// =========================================================================
// BLAKE3 tests (Rust-only -- no standard Java equivalent)
// =========================================================================

#[test]
fn test_blake3_output_length() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = write_test_file(dir.path());

    let hash = gradle_substrate_daemon::server::hash::hash_file_blake3(&test_file).unwrap();
    assert_eq!(hash.len(), 32, "BLAKE3 should produce a 32-byte hash");
}

#[test]
fn test_blake3_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("blake3_a.bin");
    let file2 = dir.path().join("blake3_b.bin");
    let data = b"BLAKE3 determinism test content here!";
    fs::write(&file1, data).unwrap();
    fs::write(&file2, data).unwrap();

    let hash1 = gradle_substrate_daemon::server::hash::hash_file_blake3(&file1).unwrap();
    let hash2 = gradle_substrate_daemon::server::hash::hash_file_blake3(&file2).unwrap();
    assert_eq!(hash1, hash2, "BLAKE3 should be deterministic across calls");
    assert_eq!(hash1.len(), 32);
}

#[test]
fn test_blake3_differs_from_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("compare.bin");
    fs::write(&file, b"same content, different algorithms").unwrap();

    let blake3_hash = gradle_substrate_daemon::server::hash::hash_file_blake3(&file).unwrap();
    let sha256_hash = gradle_substrate_daemon::server::hash::hash_file_sha256(&file).unwrap();

    assert_eq!(blake3_hash.len(), 32);
    assert_eq!(sha256_hash.len(), 32);
    assert_ne!(
        blake3_hash, sha256_hash,
        "BLAKE3 and SHA-256 must produce different hashes for the same content"
    );
}

#[test]
fn test_blake3_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.bin");
    fs::write(&empty, "").unwrap();

    let hash = gradle_substrate_daemon::server::hash::hash_file_blake3(&empty).unwrap();
    assert_eq!(hash.len(), 32);
    // BLAKE3 of empty input is a known constant
    assert_eq!(
        format_hash(&hash),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        "BLAKE3 of empty file should match the known digest"
    );
}

// =========================================================================
// SHA3-256 and SHA3-512 tests (Rust-only, verify known empty digests)
// =========================================================================

#[test]
fn test_sha3_256_output_length() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = write_test_file(dir.path());

    let hash = gradle_substrate_daemon::server::hash::hash_file_sha3_256(&test_file).unwrap();
    assert_eq!(hash.len(), 32, "SHA3-256 should produce a 32-byte hash");
}

#[test]
fn test_sha3_256_empty_file_known_value() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.bin");
    fs::write(&empty, "").unwrap();

    let hash = gradle_substrate_daemon::server::hash::hash_file_sha3_256(&empty).unwrap();
    assert_eq!(hash.len(), 32);
    assert_eq!(
        format_hash(&hash),
        "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a",
        "SHA3-256 of empty file should match the known digest"
    );
}

#[test]
fn test_sha3_512_output_length() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = write_test_file(dir.path());

    let hash = gradle_substrate_daemon::server::hash::hash_file_sha3_512(&test_file).unwrap();
    assert_eq!(hash.len(), 64, "SHA3-512 should produce a 64-byte hash");
}

#[test]
fn test_sha3_512_empty_file_known_value() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.bin");
    fs::write(&empty, "").unwrap();

    let hash = gradle_substrate_daemon::server::hash::hash_file_sha3_512(&empty).unwrap();
    assert_eq!(hash.len(), 64);
    assert_eq!(
        format_hash(&hash),
        "a69f73cca23a9ac5c8b567dc185a756e97c982164fe25859e0d1dcc1475c80a615b2123af1f5f94c11e3e9402c3ac558f500199d95b6d3e301758586281dcd26",
        "SHA3-512 of empty file should match the known digest"
    );
}

// =========================================================================
// All 6 algorithms produce different hashes for the same content
// =========================================================================

#[test]
fn test_all_algorithms_produce_different_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("multi_algo.bin");
    fs::write(&file, b"test content for all six algorithms").unwrap();

    let md5 = gradle_substrate_daemon::server::hash::hash_file_md5(&file).unwrap();
    let sha1 = gradle_substrate_daemon::server::hash::hash_file_sha1(&file).unwrap();
    let sha256 = gradle_substrate_daemon::server::hash::hash_file_sha256(&file).unwrap();
    let sha3_256 = gradle_substrate_daemon::server::hash::hash_file_sha3_256(&file).unwrap();
    let sha3_512 = gradle_substrate_daemon::server::hash::hash_file_sha3_512(&file).unwrap();
    let blake3 = gradle_substrate_daemon::server::hash::hash_file_blake3(&file).unwrap();

    // Verify expected lengths
    assert_eq!(md5.len(), 16);
    assert_eq!(sha1.len(), 20);
    assert_eq!(sha256.len(), 32);
    assert_eq!(sha3_256.len(), 32);
    assert_eq!(sha3_512.len(), 64);
    assert_eq!(blake3.len(), 32);

    // All should differ from each other (MD5 has signature wrapping so it's definitely different)
    // For the no-signature algorithms, compare among themselves
    let no_sig_hashes = [&sha1, &sha256, &sha3_256, &sha3_512, &blake3];
    for i in 0..no_sig_hashes.len() {
        for j in (i + 1)..no_sig_hashes.len() {
            // Truncate to min length for comparison where sizes differ
            let min_len = no_sig_hashes[i].len().min(no_sig_hashes[j].len());
            assert_ne!(
                &no_sig_hashes[i][..min_len],
                &no_sig_hashes[j][..min_len],
                "Algorithms should produce different hashes (pair {}, {})",
                i,
                j
            );
        }
    }

    // MD5 should also differ from all others
    for h in &no_sig_hashes {
        assert_ne!(
            &md5[..],
            &h[..std::cmp::min(md5.len(), h.len())],
            "MD5 should differ from other algorithm outputs"
        );
    }
}

// =========================================================================
// Binary content test (random bytes) with MD5
// =========================================================================

#[test]
fn test_md5_binary_content_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let binary_file = dir.path().join("binary.bin");

    // Generate a deterministic pseudo-random byte sequence using a simple LFSR-like pattern.
    // This avoids needing a real RNG while producing non-trivial binary data.
    let data: Vec<u8> = (0..2048)
        .map(|i| {
            let v = (((i as u64).wrapping_mul(1103515245).wrapping_add(12345)) >> 16) & 0xFF;
            v as u8
        })
        .collect();
    fs::write(&binary_file, &data).unwrap();

    let rust_hash = compute_rust_hash(&binary_file);
    let java_hash = compute_java_hash(&binary_file);

    assert_eq!(rust_hash.len(), 16);
    assert_eq!(java_hash.len(), 16);
    assert_eq!(
        rust_hash,
        java_hash,
        "MD5 binary content: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_sha256_binary_content_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let binary_file = dir.path().join("binary_sha256.bin");

    // Deterministic pseudo-random binary data
    let data: Vec<u8> = (0..5000)
        .map(|i| {
            let v = (((i as u64)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407))
                >> 24)
                & 0xFF;
            v as u8
        })
        .collect();
    fs::write(&binary_file, &data).unwrap();

    let rust_hash = gradle_substrate_daemon::server::hash::hash_file_sha256(&binary_file).unwrap();
    let java_hash = compute_java_sha256(&binary_file);

    assert_eq!(rust_hash.len(), 32);
    assert_eq!(java_hash.len(), 32);
    assert_eq!(
        rust_hash,
        java_hash,
        "SHA-256 binary content: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

// =========================================================================
// Exact 8192-byte boundary test (chunk boundary in Java's DefaultStreamHasher)
// =========================================================================

#[test]
fn test_md5_exact_8192_byte_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let boundary_file = dir.path().join("boundary.bin");

    // Exactly 8192 bytes: Java's DefaultStreamHasher uses 8192-byte buffer.
    // This file fills exactly one buffer with no remainder.
    let data = vec![0xABu8; 8192];
    fs::write(&boundary_file, &data).unwrap();

    let rust_hash = compute_rust_hash(&boundary_file);
    let java_hash = compute_java_hash(&boundary_file);

    assert_eq!(rust_hash.len(), 16);
    assert_eq!(java_hash.len(), 16);
    assert_eq!(
        rust_hash,
        java_hash,
        "MD5 at 8192-byte boundary: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_md5_one_byte_past_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("boundary_plus1.bin");

    // 8193 bytes: one byte past the exact buffer boundary
    let data = vec![0xCDu8; 8193];
    fs::write(&file, &data).unwrap();

    let rust_hash = compute_rust_hash(&file);
    let java_hash = compute_java_hash(&file);

    assert_eq!(
        rust_hash,
        java_hash,
        "MD5 at 8193 bytes: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_md5_one_byte_before_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("boundary_minus1.bin");

    // 8191 bytes: one byte before the exact buffer boundary
    let data = vec![0xEFu8; 8191];
    fs::write(&file, &data).unwrap();

    let rust_hash = compute_rust_hash(&file);
    let java_hash = compute_java_hash(&file);

    assert_eq!(
        rust_hash,
        java_hash,
        "MD5 at 8191 bytes: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_sha256_exact_8192_byte_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let boundary_file = dir.path().join("boundary_sha256.bin");

    let data = vec![0x37u8; 8192];
    fs::write(&boundary_file, &data).unwrap();

    let rust_hash =
        gradle_substrate_daemon::server::hash::hash_file_sha256(&boundary_file).unwrap();
    let java_hash = compute_java_sha256(&boundary_file);

    assert_eq!(rust_hash.len(), 32);
    assert_eq!(java_hash.len(), 32);
    assert_eq!(
        rust_hash,
        java_hash,
        "SHA-256 at 8192-byte boundary: Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

// =========================================================================
// Multi-chunk file test (8192 * 3 + 42 = 24618 bytes)
// =========================================================================

#[test]
fn test_md5_multi_chunk_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let multi_chunk_file = dir.path().join("multi_chunk.bin");

    // 8192 * 3 + 42 = 24618 bytes: fills 3 full 8192-byte chunks in Java plus a
    // 42-byte remainder, testing multi-chunk streaming correctness.
    let total_len = 8192 * 3 + 42;
    let data: Vec<u8> = (0..total_len as u64)
        .map(|i| ((i * 7 + 13) % 256) as u8)
        .collect();
    assert_eq!(data.len(), total_len);
    fs::write(&multi_chunk_file, &data).unwrap();

    let rust_hash = compute_rust_hash(&multi_chunk_file);
    let java_hash = compute_java_hash(&multi_chunk_file);

    assert_eq!(rust_hash.len(), 16);
    assert_eq!(java_hash.len(), 16);
    assert_eq!(
        rust_hash,
        java_hash,
        "MD5 multi-chunk (3*8192+42): Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_sha256_multi_chunk_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let multi_chunk_file = dir.path().join("multi_chunk_sha256.bin");

    let total_len = 8192 * 3 + 42;
    let data: Vec<u8> = (0..total_len as u64)
        .map(|i| ((i * 11 + 7) % 256) as u8)
        .collect();
    fs::write(&multi_chunk_file, &data).unwrap();

    let rust_hash =
        gradle_substrate_daemon::server::hash::hash_file_sha256(&multi_chunk_file).unwrap();
    let java_hash = compute_java_sha256(&multi_chunk_file);

    assert_eq!(rust_hash.len(), 32);
    assert_eq!(java_hash.len(), 32);
    assert_eq!(
        rust_hash, java_hash,
        "SHA-256 multi-chunk (3*8192+42): Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

#[test]
fn test_sha1_multi_chunk_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let multi_chunk_file = dir.path().join("multi_chunk_sha1.bin");

    let total_len = 8192 * 3 + 42;
    let data: Vec<u8> = (0..total_len as u64)
        .map(|i| ((i * 3 + 17) % 256) as u8)
        .collect();
    fs::write(&multi_chunk_file, &data).unwrap();

    let rust_hash =
        gradle_substrate_daemon::server::hash::hash_file_sha1(&multi_chunk_file).unwrap();
    let java_hash = compute_java_sha1(&multi_chunk_file);

    assert_eq!(rust_hash.len(), 20);
    assert_eq!(java_hash.len(), 20);
    assert_eq!(
        rust_hash,
        java_hash,
        "SHA-1 multi-chunk (3*8192+42): Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        format_hash(&rust_hash),
        format_hash(&java_hash)
    );
}

// =========================================================================
// All algorithms on multi-chunk data (Rust-only consistency)
// =========================================================================

#[test]
fn test_all_algorithms_multi_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("all_algo_multichunk.bin");

    let total_len = 8192 * 3 + 42;
    let data: Vec<u8> = (0..total_len as u64)
        .map(|i| ((i * 13 + 23) % 256) as u8)
        .collect();
    fs::write(&file, &data).unwrap();

    let md5 = gradle_substrate_daemon::server::hash::hash_file_md5(&file).unwrap();
    let sha1 = gradle_substrate_daemon::server::hash::hash_file_sha1(&file).unwrap();
    let sha256 = gradle_substrate_daemon::server::hash::hash_file_sha256(&file).unwrap();
    let sha3_256 = gradle_substrate_daemon::server::hash::hash_file_sha3_256(&file).unwrap();
    let sha3_512 = gradle_substrate_daemon::server::hash::hash_file_sha3_512(&file).unwrap();
    let blake3 = gradle_substrate_daemon::server::hash::hash_file_blake3(&file).unwrap();

    // Verify expected lengths
    assert_eq!(md5.len(), 16);
    assert_eq!(sha1.len(), 20);
    assert_eq!(sha256.len(), 32);
    assert_eq!(sha3_256.len(), 32);
    assert_eq!(sha3_512.len(), 64);
    assert_eq!(blake3.len(), 32);

    // Verify none are all-zeros (sanity check that the hash is computed)
    assert!(md5.iter().any(|&b| b != 0));
    assert!(sha1.iter().any(|&b| b != 0));
    assert!(sha256.iter().any(|&b| b != 0));
    assert!(sha3_256.iter().any(|&b| b != 0));
    assert!(sha3_512.iter().any(|&b| b != 0));
    assert!(blake3.iter().any(|&b| b != 0));
}
