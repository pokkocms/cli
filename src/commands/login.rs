use anyhow::{bail, Context, Result};
use colored::Colorize;
use rand::Rng;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::credentials;

const LOGIN_TIMEOUT_SECS: u64 = 300; // 5 minutes

pub async fn run(auth_url: &str) -> Result<()> {
    // Generate a random state token to prevent CSRF
    let state: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    // Bind on a random available port
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind local callback server")?;
    let port = listener.local_addr()?.port();

    let redirect_uri = format!("http://localhost:{}/callback", port);
    let login_url = format!(
        "{}/cli-login?redirect_uri={}&state={}",
        auth_url.trim_end_matches('/'),
        urlencoding_encode(&redirect_uri),
        state
    );

    println!("{}", "Opening your browser to log in...".bold());
    println!();
    println!("If the browser doesn't open, visit this URL manually:");
    println!("  {}", login_url.cyan());
    println!();

    // Best-effort browser open
    let _ = open::that(&login_url);

    println!("Waiting for authentication (timeout: {}s)...", LOGIN_TIMEOUT_SECS);

    let result = tokio::time::timeout(
        Duration::from_secs(LOGIN_TIMEOUT_SECS),
        wait_for_callback(listener, &state),
    )
    .await;

    match result {
        Ok(Ok(token)) => {
            credentials::save(&credentials::Credentials {
                token,
                auth_url: auth_url.to_string(),
            })?;
            println!();
            println!("{}", "Logged in successfully.".green().bold());
            println!("Credentials saved to ~/.pokko/credentials.toml");
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => bail!("login timed out — please try again"),
    }
}

async fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    let (mut stream, _) = listener
        .accept()
        .await
        .context("failed to accept callback connection")?;

    // Read the HTTP request
    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .context("failed to read callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the first line: "GET /callback?token=...&state=... HTTP/1.1"
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("");

    let query = path.splitn(2, '?').nth(1).unwrap_or("");
    let params = parse_query(query);

    // Always send a response so the browser doesn't hang
    let (body, status) = match extract_token(&params, expected_state) {
        Ok(_) => (
            "<html><body><h2>Login successful!</h2><p>You can close this tab and return to the terminal.</p></body></html>",
            "200 OK",
        ),
        Err(_) => (
            "<html><body><h2>Login failed.</h2><p>Please return to the terminal and try again.</p></body></html>",
            "400 Bad Request",
        ),
    };

    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    extract_token(&params, expected_state)
}

fn extract_token(params: &std::collections::HashMap<String, String>, expected_state: &str) -> Result<String> {
    let returned_state = params.get("state").map(|s| s.as_str()).unwrap_or("");
    if returned_state != expected_state {
        bail!("state mismatch — possible CSRF attack, login aborted");
    }
    params
        .get("token")
        .cloned()
        .context("no token in callback — login failed")
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let k = parts.next()?;
            let v = parts.next().unwrap_or("");
            Some((url_decode(k), url_decode(v)))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b as char);
                    i += 3;
                    continue;
                }
            }
        }
        if bytes[i] == b'+' {
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b':' => out.push(':'),
            b'/' => out.push('/'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
