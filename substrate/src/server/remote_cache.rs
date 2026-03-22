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
                        let bytes = resp.bytes().await
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
            let mut request = self.client.put(&url)
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
        let store = RemoteCacheStore::new(
            "https://example.com".to_string(),
            None,
            None,
        );
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
        let store = RemoteCacheStore::new(
            "https://example.com/cache/".to_string(),
            None,
            None,
        );
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
}
