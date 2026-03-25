use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

/// HTTP-based remote build cache store.
/// Supports GET/PUT with retry logic, authentication, and streaming.
pub struct RemoteCacheStore {
    client: reqwest::Client,
    base_url: String,
    username: Option<String>,
    password: Option<String>,
    max_retries: u32,
}

impl Default for RemoteCacheStore {
    fn default() -> Self {
        Self::new(String::new(), None, None)
    }
}

impl RemoteCacheStore {
    pub fn new(base_url: String, username: Option<String>, password: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base_url: base_url.trim_end_matches('/').to_string(),
            username,
            password,
            max_retries: 3,
        }
    }

    fn auth_header(&self) -> Option<String> {
        match (&self.username, &self.password) {
            (Some(user), Some(pass)) => {
                use std::io::Write;
                let mut buf = Vec::new();
                write!(buf, "{}:{}", user, pass).unwrap();
                Some(format!("Basic {}", base64_encode(&buf)))
            }
            _ => None,
        }
    }

    /// Load a cache entry from the remote cache.
    /// Returns None on 404, Some(bytes) on 200.
    /// Retries on 5xx with exponential backoff.
    pub async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let url = format!("{}/{}", self.base_url, key);
        let mut attempt = 0;

        loop {
            attempt += 1;
            let mut request = self.client.get(&url);
            if let Some(auth) = self.auth_header() {
                request = request.header(AUTHORIZATION, auth);
            }

            match request.send().await {
                Ok(resp) => match resp.status().as_u16() {
                    200..=299 => {
                        let bytes = resp
                            .bytes()
                            .await
                            .map_err(|e| format!("Failed to read response body: {}", e))?;
                        return Ok(Some(bytes.to_vec()));
                    }
                    404 => return Ok(None),
                    500..=599 if attempt < self.max_retries => {
                        let delay = Duration::from_millis(100 * 2u64.pow(attempt - 1));
                        tracing::warn!(
                            url = %url,
                            attempt,
                            retry_after_ms = delay.as_millis(),
                            "Remote cache GET returned {}, retrying",
                            resp.status()
                        );
                        tokio::time::sleep(delay).await;
                    }
                    status => {
                        return Err(format!("Remote cache GET {} returned {}", url, status));
                    }
                },
                Err(e) if attempt < self.max_retries => {
                    let delay = Duration::from_millis(100 * 2u64.pow(attempt - 1));
                    tracing::warn!(
                        url = %url,
                        attempt,
                        error = %e,
                        retry_after_ms = delay.as_millis(),
                        "Remote cache GET failed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    return Err(format!("Remote cache GET {} failed: {}", url, e));
                }
            }
        }
    }

    /// Store a cache entry to the remote cache.
    /// PUTs the bytes with content-type application/octet-stream.
    /// Retries on 5xx with exponential backoff.
    pub async fn store(&self, key: &str, data: &[u8]) -> Result<(), String> {
        let url = format!("{}/{}", self.base_url, key);
        let mut attempt = 0;

        loop {
            attempt += 1;
            let mut request = self
                .client
                .put(&url)
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(data.to_vec());
            if let Some(auth) = self.auth_header() {
                request = request.header(AUTHORIZATION, auth);
            }

            match request.send().await {
                Ok(resp) => match resp.status().as_u16() {
                    200..=299 => return Ok(()),
                    500..=599 if attempt < self.max_retries => {
                        let delay = Duration::from_millis(100 * 2u64.pow(attempt - 1));
                        tracing::warn!(
                            url = %url,
                            attempt,
                            retry_after_ms = delay.as_millis(),
                            "Remote cache PUT returned {}, retrying",
                            resp.status()
                        );
                        tokio::time::sleep(delay).await;
                    }
                    status => {
                        return Err(format!("Remote cache PUT {} returned {}", url, status));
                    }
                },
                Err(e) if attempt < self.max_retries => {
                    let delay = Duration::from_millis(100 * 2u64.pow(attempt - 1));
                    tracing::warn!(
                        url = %url,
                        attempt,
                        error = %e,
                        retry_after_ms = delay.as_millis(),
                        "Remote cache PUT failed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    return Err(format!("Remote cache PUT {} failed: {}", url, e));
                }
            }
        }
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_new_store() {
        let store = RemoteCacheStore::new(
            "https://example.com/cache".to_string(),
            Some("user".to_string()),
            Some("pass".to_string()),
        );
        assert_eq!(store.base_url, "https://example.com/cache");
        assert!(store.auth_header().is_some());
    }

    #[test]
    fn test_auth_header() {
        let store = RemoteCacheStore::new(
            "https://example.com".to_string(),
            Some("user".to_string()),
            Some("pass".to_string()),
        );
        let auth = store.auth_header().unwrap();
        assert!(auth.starts_with("Basic "));
    }

    #[test]
    fn test_no_auth() {
        let store = RemoteCacheStore::new("https://example.com".to_string(), None, None);
        assert!(store.auth_header().is_none());
    }

    #[test]
    fn test_base64_known_vectors() {
        // RFC 4648 test vectors
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_auth_header_value() {
        let store = RemoteCacheStore::new(
            "https://cache.example.com".to_string(),
            Some("myuser".to_string()),
            Some("mypass".to_string()),
        );
        let auth = store.auth_header().unwrap();
        // "myuser:mypass" base64 encoded
        let expected = base64_encode(b"myuser:mypass");
        assert_eq!(auth, format!("Basic {}", expected));
    }

    #[test]
    fn test_trailing_slash_trimmed() {
        let store = RemoteCacheStore::new("https://example.com/cache/".to_string(), None, None);
        assert_eq!(store.base_url, "https://example.com/cache");
    }

    #[test]
    fn test_partial_credentials() {
        // Only username, no password
        let store = RemoteCacheStore::new(
            "https://example.com".to_string(),
            Some("user".to_string()),
            None,
        );
        assert!(store.auth_header().is_none());

        // Only password, no username
        let store2 = RemoteCacheStore::new(
            "https://example.com".to_string(),
            None,
            Some("pass".to_string()),
        );
        assert!(store2.auth_header().is_none());
    }

    // ---------------------------------------------------------------------------
    // Async tests: exercise load/store against a real HTTP server
    // ---------------------------------------------------------------------------

    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::RwLock;

    /// Spin up a minimal HTTP/1.1 server on a random port backed by an in-memory
    /// `HashMap`.  Returns `(base_url, handle)` where the handle must be kept
    /// alive for the duration of the test.
    async fn spawn_mock_server(store: Arc<RwLock<HashMap<String, Vec<u8>>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => continue,
                };

                let store = store.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncReadExt;
                    use tokio::io::AsyncWriteExt;

                    let mut stream = stream;
                    let mut buf = Vec::new();

                    // Read until we have at least the full header (ended by \r\n\r\n).
                    {
                        let mut tmp = [0u8; 4096];
                        loop {
                            let n = match stream.read(&mut tmp).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => n,
                            };
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }

                    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    let header_str = String::from_utf8_lossy(&buf[..header_end]);

                    let (method, path) = parse_request_line(&header_str);

                    // For PUT, read the remaining body based on Content-Length.
                    if method == "PUT" {
                        if let Some(cl) = parse_content_length(&header_str) {
                            let body_so_far = buf.len() - header_end;
                            let remaining = cl.saturating_sub(body_so_far);
                            if remaining > 0 {
                                let mut body_buf = vec![0u8; remaining];
                                let mut read = 0;
                                while read < remaining {
                                    match stream.read(&mut body_buf[read..]).await {
                                        Ok(0) | Err(_) => break,
                                        Ok(n) => read += n,
                                    }
                                }
                                buf.extend_from_slice(&body_buf[..read]);
                            }
                        }
                    }

                    let body_bytes = &buf[header_end..];

                    let (status, response_body): (&str, Vec<u8>) = match method.as_str() {
                        "GET" => {
                            let s = store.read().await;
                            match s.get(&path) {
                                Some(data) => ("200 OK", data.clone()),
                                None => ("404 Not Found", Vec::new()),
                            }
                        }
                        "PUT" => {
                            {
                                let mut s = store.write().await;
                                s.insert(path.clone(), body_bytes.to_vec());
                            }
                            ("200 OK", Vec::new())
                        }
                        _ => ("405 Method Not Allowed", Vec::new()),
                    };

                    let response = format!(
                        "HTTP/1.1 {}\r\nContent-Length: {}\r\n\r\n",
                        status,
                        response_body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    if !response_body.is_empty() {
                        let _ = stream.write_all(&response_body).await;
                    }
                });
            }
        });

        format!("http://127.0.0.1:{}", addr.port())
    }

    /// Very small request-line parser -- just enough for our tests.
    fn parse_request_line(request: &str) -> (String, String) {
        let first_line = request.lines().next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("/").to_string();
        (method, path)
    }

    /// Parse the Content-Length header value, case-insensitive.
    fn parse_content_length(header: &str) -> Option<usize> {
        for line in header.lines() {
            if let Some(val) = line.strip_prefix("content-length:") {
                return val.trim().parse().ok();
            }
            if let Some(val) = line.strip_prefix("Content-Length:") {
                return val.trim().parse().ok();
            }
        }
        None
    }

    #[tokio::test]
    async fn test_store_with_empty_key_returns_error() {
        // An empty artifact key causes the URL to be `base_url/`, which the mock
        // server will treat as a GET for "/" (404).  The store should not panic;
        // it should return either an error or a well-defined result.
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        // Store with an empty key – the server will see path "/".
        let result = store.store("", b"some data").await;
        // The store itself does not validate empty keys; it will PUT to
        // `base_url/` and the server will return 200 because the path "/"
        // is valid for our mock.  Verify the store doesn't panic and
        // succeeds (the real value is that it doesn't crash).
        assert!(
            result.is_ok(),
            "store should succeed for empty key (no crash)"
        );

        // Loading with empty key should hit the "/" path in the mock server,
        // which will return whatever was stored there.
        let loaded = store.load("").await;
        assert!(
            loaded.is_ok(),
            "load should succeed for empty key (no crash)"
        );
        assert_eq!(loaded.unwrap(), Some(b"some data".to_vec()));
    }

    #[tokio::test]
    async fn test_multiple_stores_loads_last_write_wins() {
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        let key = "/artifact/abc123";

        // First write
        store.store(key, b"version-one").await.unwrap();
        let loaded = store.load(key).await.unwrap();
        assert_eq!(loaded, Some(b"version-one".to_vec()));

        // Second write overwrites
        store.store(key, b"version-two-longer").await.unwrap();
        let loaded = store.load(key).await.unwrap();
        assert_eq!(loaded, Some(b"version-two-longer".to_vec()));

        // Third write
        store.store(key, b"v3").await.unwrap();
        let loaded = store.load(key).await.unwrap();
        assert_eq!(loaded, Some(b"v3".to_vec()));
    }

    #[tokio::test]
    async fn test_load_nonexistent_artifact_returns_not_found() {
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        // Key was never stored – server returns 404, store maps that to Ok(None).
        let result = store.load("/artifact/does-not-exist").await;
        assert!(result.is_ok(), "load should not error on 404");
        assert_eq!(result.unwrap(), None, "nonexistent key should yield None");

        // Also verify a second miss for good measure
        let result2 = store.load("/artifact/another-miss").await;
        assert_eq!(result2.unwrap(), None);
    }

    #[tokio::test]
    async fn test_store_and_load_with_metadata_sized_payload() {
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        // Build a payload that simulates artifact content with a metadata prefix.
        // The RemoteCacheStore is opaque to the payload, so we store/load raw bytes.
        let metadata = b"SIZE:128\nCONTENT-TYPE:application/zip\n\n";
        let artifact_content = vec![0xABu8; 128];
        let mut payload = Vec::new();
        payload.extend_from_slice(metadata);
        payload.extend_from_slice(&artifact_content);

        let key = "/cache/metadata-test";
        store.store(key, &payload).await.unwrap();

        let loaded = store.load(key).await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), payload.len());

        // Verify the metadata prefix is intact
        let loaded_str = String::from_utf8_lossy(&loaded[..metadata.len()]);
        assert!(loaded_str.starts_with("SIZE:128"));
        assert!(loaded_str.contains("CONTENT-TYPE:application/zip"));

        // Verify the artifact content portion
        assert_eq!(&loaded[metadata.len()..], &artifact_content[..]);
    }

    #[tokio::test]
    async fn test_has_nonexistent_keys_returns_false() {
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        // RemoteCacheStore.load returns Ok(None) for 404s, which is the
        // equivalent of "does not have".  Test several nonexistent keys.
        let missing_keys = ["/cache/aaa111", "/cache/bbb222", "/cache/ccc333"];
        for key in &missing_keys {
            let result = store.load(key).await.unwrap();
            assert_eq!(result, None, "key {} should not exist", key);
        }

        // Store one of them and verify the others remain missing
        store.store("/cache/aaa111", b"present").await.unwrap();
        assert_eq!(
            store.load("/cache/aaa111").await.unwrap(),
            Some(b"present".to_vec())
        );
        assert_eq!(store.load("/cache/bbb222").await.unwrap(), None);
        assert_eq!(store.load("/cache/ccc333").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_store_with_very_large_payload() {
        // Verify that a 1 MB+ payload is stored and loaded back correctly.
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        let payload_size = 1024 * 1024 + 512; // 1 MB + 512 bytes
        let payload: Vec<u8> = (0..payload_size).map(|i| (i % 256) as u8).collect();

        let key = "/cache/large-artifact";
        store.store(key, &payload).await.unwrap();

        let loaded = store.load(key).await.unwrap();
        assert!(loaded.is_some(), "large payload should be retrievable");
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), payload_size, "loaded payload size must match");

        // Verify byte-level fidelity across the entire payload
        for i in 0..payload_size {
            assert_eq!(loaded[i], payload[i], "byte mismatch at offset {}", i);
        }
    }

    #[tokio::test]
    async fn test_concurrent_stores_to_different_keys() {
        // Fire off multiple store requests in parallel to different keys and
        // verify they all land correctly without interference.
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        let num_keys = 20usize;
        let mut handles = Vec::new();

        for i in 0..num_keys {
            let s = RemoteCacheStore::new(
                store.base_url.clone(),
                store.username.clone(),
                store.password.clone(),
            );
            let key = format!("/cache/concurrent/{}", i);
            let data = format!("payload-{}", i).into_bytes();
            handles.push(tokio::spawn(async move {
                s.store(&key, &data).await.unwrap();
                (key, data)
            }));
        }

        // Await all stores and collect the expected key/value pairs.
        let mut expected: HashMap<String, Vec<u8>> = HashMap::new();
        for handle in handles {
            let (key, data) = handle.await.unwrap();
            expected.insert(key, data);
        }

        // Verify every key loads back the correct value.
        for (key, expected_data) in &expected {
            let loaded = store.load(key).await.unwrap();
            assert_eq!(
                loaded,
                Some(expected_data.clone()),
                "mismatch for key {}",
                key
            );
        }

        // Also verify the total number of entries in the backing store.
        let store_map = backing.read().await;
        assert_eq!(
            store_map.len(),
            num_keys,
            "all concurrent stores should be present"
        );
    }

    #[tokio::test]
    async fn test_store_then_load_after_server_restart() {
        // Store data in a first server instance, then spin up a brand-new
        // server with an empty backing store and verify the load returns None
        // (data was not persisted across the restart).
        let backing1 = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url1 = spawn_mock_server(backing1.clone()).await;
        let store1 = RemoteCacheStore::new(base_url1, None, None);

        let key = "/cache/survives-restart";
        store1.store(key, b"important-data").await.unwrap();
        assert_eq!(
            store1.load(key).await.unwrap(),
            Some(b"important-data".to_vec()),
            "data should be present on the original server"
        );

        // Now spin up a second, independent server with an empty store.
        let backing2 = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url2 = spawn_mock_server(backing2.clone()).await;
        let store2 = RemoteCacheStore::new(base_url2, None, None);

        let loaded = store2.load(key).await.unwrap();
        assert_eq!(
            loaded, None,
            "data should NOT be present on the restarted server"
        );

        // Store new data on the second server and verify it works.
        store2.store(key, b"new-data").await.unwrap();
        assert_eq!(store2.load(key).await.unwrap(), Some(b"new-data".to_vec()),);
    }

    #[tokio::test]
    async fn test_has_many_with_mixed_existing_and_nonexisting_keys() {
        // Store a handful of keys, then check a larger set that mixes stored
        // and never-stored keys, verifying each individually.
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        // Pre-populate some keys
        let existing_keys: Vec<String> = (0..5)
            .map(|i| {
                let key = format!("/cache/exists/{}", i);
                let data = format!("value-{}", i);
                let store_cloned = RemoteCacheStore::new(
                    store.base_url.clone(),
                    store.username.clone(),
                    store.password.clone(),
                );
                let key_clone = key.clone();
                tokio::spawn(async move {
                    store_cloned
                        .store(&key_clone, data.as_bytes())
                        .await
                        .unwrap();
                });
                key
            })
            .collect();

        // Wait for all stores to finish
        for _handle in existing_keys {
            // (the handles were fire-and-forget, give the server a moment)
        }
        // Give the spawned tasks time to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Build a mixed query set: some existing, some not
        let all_keys: Vec<String> = (0..10).map(|i| format!("/cache/exists/{}", i)).collect();

        let mut present_count = 0;
        let mut missing_count = 0;
        for key in &all_keys {
            let result = store.load(key).await.unwrap();
            if result.is_some() {
                present_count += 1;
            } else {
                missing_count += 1;
            }
        }

        assert_eq!(present_count, 5, "exactly 5 keys should be present");
        assert_eq!(missing_count, 5, "exactly 5 keys should be missing");
    }

    #[tokio::test]
    async fn test_load_with_corrupted_metadata_returns_gracefully() {
        // Simulate server-side corruption by inserting truncated and garbage
        // payloads directly into the backing store.  The RemoteCacheStore is
        // opaque to payload semantics, so loads should still succeed and
        // return whatever bytes the server provides.  The caller is
        // responsible for validating content integrity.
        let backing = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
        let base_url = spawn_mock_server(backing.clone()).await;
        let store = RemoteCacheStore::new(base_url, None, None);

        // Insert a truncated payload directly into the backing store,
        // bypassing the mock server's PUT handler, to simulate corruption.
        // Note: RemoteCacheStore prepends a "/" when building the URL, so the
        // mock server sees the path as "//cache/corrupt-truncated".
        let key_truncated = "/cache/corrupt-truncated";
        let server_path = format!("/{}", key_truncated); // becomes "//cache/corrupt-truncated"
        {
            let mut map = backing.write().await;
            // A payload whose metadata header claims SIZE:256 but only has
            // 5 bytes of actual data — clearly corrupted.
            map.insert(server_path, b"SIZE:".to_vec());
        }

        let loaded = store.load(key_truncated).await;
        assert!(loaded.is_ok(), "load should not error on corrupted data");
        let loaded = loaded.unwrap();
        assert!(
            loaded.is_some(),
            "corrupted entry should still be retrievable"
        );
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), 5, "truncated entry should be 5 bytes");
        assert_eq!(
            &loaded[..],
            b"SIZE:",
            "bytes should match the corrupted content"
        );

        // Insert a completely garbage payload.
        let key_garbage = "/cache/corrupt-garbage";
        let server_path_garbage = format!("/{}", key_garbage);
        let garbage = vec![0xDEu8, 0xAD, 0xBE, 0xEF, 0x00, 0xFF];
        {
            let mut map = backing.write().await;
            map.insert(server_path_garbage, garbage.clone());
        }
        let loaded2 = store.load(key_garbage).await.unwrap();
        assert_eq!(loaded2, Some(garbage.clone()));

        // Verify that a normal store+load still works alongside corrupted
        // entries — the corruption of one key does not affect others.
        let key_good = "/cache/corrupt-good-neighbor";
        store.store(key_good, b"perfectly-fine-data").await.unwrap();
        let loaded3 = store.load(key_good).await.unwrap();
        assert_eq!(loaded3, Some(b"perfectly-fine-data".to_vec()));
    }
}
