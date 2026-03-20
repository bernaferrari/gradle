use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Cross-language hash compatibility test.
///
/// Writes a known file, computes its hash using Rust's implementation,
/// then computes the same hash using Java's DefaultStreamHasher via a small Java program,
/// and compares the results.

const TEST_CONTENT: &[u8] = b"Hello, Gradle Rust Substrate! This is a test file for cross-language hash verification.\n";

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
        panic!("Java compilation failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let output = Command::new("java")
        .arg("-cp")
        .arg(dir.path())
        .arg("ComputeHash")
        .output()
        .expect("Failed to run Java hash program");

    if !output.status.success() {
        panic!("Java execution failed: {}", String::from_utf8_lossy(&output.stderr));
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

#[test]
fn test_cross_language_hash_compatibility() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = write_test_file(dir.path());

    let rust_hash = compute_rust_hash(&test_file);
    let java_hash = compute_java_hash(&test_file);

    assert_eq!(rust_hash.len(), 16, "Rust hash should be 16 bytes (MD5)");
    assert_eq!(java_hash.len(), 16, "Java hash should be 16 bytes (MD5)");
    assert_eq!(
        rust_hash, java_hash,
        "Rust and Java hashes should match.\n  Rust: {}\n  Java: {}",
        rust_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(),
        java_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()
    );
}

#[test]
fn test_empty_file_cross_language() {
    let dir = tempfile::tempdir().unwrap();
    let empty_file = dir.path().join("empty.txt");
    fs::write(&empty_file, "").unwrap();

    let rust_hash = compute_rust_hash(&empty_file);
    let java_hash = compute_java_hash(&empty_file);

    assert_eq!(rust_hash, java_hash,
        "Empty file hashes should match.\n  Rust: {}\n  Java: {}",
        rust_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(),
        java_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()
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

    assert_eq!(rust_hash, java_hash,
        "Large file hashes should match.\n  Rust: {}\n  Java: {}",
        rust_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(),
        java_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()
    );
}
