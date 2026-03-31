//! Minimal LSP server scaffold for Gradle build script editing.
//!
//! Supports initialize and textDocument/didOpen as stubs.
//! Parses .gradle and .gradle.kts files using existing parsers
//! to provide syntax highlighting data.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Read, Write};

/// LSP initialize response capabilities.
#[derive(Debug, Serialize)]
struct ServerCapabilities {
    text_document_sync: TextDocumentSync,
    completion_provider: CompletionProvider,
}

#[derive(Debug, Serialize)]
struct TextDocumentSync {
    open_close: bool,
}

#[derive(Debug, Serialize)]
struct CompletionProvider {
    trigger_characters: Vec<String>,
}

/// LSP initialize response.
#[derive(Debug, Serialize)]
struct InitializeResult {
    capabilities: ServerCapabilities,
    server_info: ServerInfo,
}

#[derive(Debug, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

/// LSP request envelope.
#[derive(Debug, Deserialize)]
struct LspRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// LSP response envelope.
#[derive(Debug, Serialize)]
struct LspResponse {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<LspError>,
}

#[derive(Debug, Serialize)]
struct LspError {
    code: i32,
    message: String,
}

/// Run the LSP server over stdio.
pub fn run_lsp_server() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        let mut line = String::new();
        let bytes_read = stdin.lock().read_line(&mut line)?;
        if bytes_read == 0 {
            break; // EOF
        }

        // Parse Content-Length header
        let line = line.trim();
        if !line.starts_with("Content-Length:") {
            continue;
        }

        let content_length: usize = line
            .strip_prefix("Content-Length:")
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(0);

        // Read empty line separator
        let mut separator = String::new();
        stdin.lock().read_line(&mut separator)?;

        // Read the JSON body
        let mut body = vec![0u8; content_length];
        io::stdin().read_exact(&mut body)?;

        let request: LspRequest = match serde_json::from_slice(&body) {
            Ok(req) => req,
            Err(_) => continue,
        };

        let response = handle_request(&request);
        let response_json = serde_json::to_string(&response)?;
        let response_bytes = response_json.as_bytes();

        writeln!(stdout, "Content-Length: {}", response_bytes.len())?;
        writeln!(stdout)?;
        stdout.write_all(response_bytes)?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle_request(request: &LspRequest) -> LspResponse {
    match request.method.as_str() {
        "initialize" => handle_initialize(request.id.as_ref()),
        "initialized" => LspResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        },
        "textDocument/didOpen" => handle_did_open(request.id.as_ref(), &request.params),
        "shutdown" => LspResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: Some(serde_json::Value::Null),
            error: None,
        },
        "exit" => {
            std::process::exit(0);
        }
        _ => LspResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: None,
            error: Some(LspError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
            }),
        },
    }
}

fn handle_initialize(id: Option<&serde_json::Value>) -> LspResponse {
    let result = InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: TextDocumentSync { open_close: true },
            completion_provider: CompletionProvider {
                trigger_characters: vec![".".to_string(), ":".to_string()],
            },
        },
        server_info: ServerInfo {
            name: "gradle-substrate-lsp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    };

    LspResponse {
        jsonrpc: "2.0".to_string(),
        id: id.cloned(),
        result: Some(serde_json::to_value(result).unwrap_or_default()),
        error: None,
    }
}

fn handle_did_open(id: Option<&serde_json::Value>, params: &serde_json::Value) -> LspResponse {
    // Extract file path from params
    if let Some(text_doc) = params.get("textDocument") {
        if let Some(uri) = text_doc.get("uri").and_then(|v| v.as_str()) {
            tracing::debug!(uri = %uri, "LSP: textDocument/didOpen");
        }
    }

    LspResponse {
        jsonrpc: "2.0".to_string(),
        id: id.cloned(),
        result: None,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_initialize() {
        let request = LspRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::Value::Number(1.into())),
            method: "initialize".to_string(),
            params: serde_json::json!({}),
        };
        let response = handle_request(&request);
        assert!(response.result.is_some());
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["server_info"]["name"], "gradle-substrate-lsp");
    }

    #[test]
    fn test_handle_unknown_method() {
        let request = LspRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::Value::Number(2.into())),
            method: "unknown/method".to_string(),
            params: serde_json::json!({}),
        };
        let response = handle_request(&request);
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[test]
    fn test_handle_shutdown() {
        let request = LspRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::Value::Number(3.into())),
            method: "shutdown".to_string(),
            params: serde_json::json!(null),
        };
        let response = handle_request(&request);
        assert!(response.result.is_some());
    }

    #[test]
    fn test_handle_did_open() {
        let request = LspRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "textDocument/didOpen".to_string(),
            params: serde_json::json!({
                "textDocument": {
                    "uri": "file:///build.gradle.kts",
                    "languageId": "kotlin",
                    "version": 1,
                    "text": "plugins { id(\"java\") }"
                }
            }),
        };
        let response = handle_request(&request);
        assert!(response.result.is_none());
        assert!(response.error.is_none());
    }
}
