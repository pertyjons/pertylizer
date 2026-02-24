//! Stdio↔TCP bridge for MCP.
//!
//! Connects Claude Code (stdio JSON-RPC) to the synth's MCP TCP server.
//! No synth dependencies — just proxies bytes between stdin/stdout and TCP.
//!
//! Usage: synth-mcp-bridge [host:port]
//! Default: 127.0.0.1:9850

use std::env;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::thread;

fn main() -> io::Result<()> {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9850".to_string());

    let stream = TcpStream::connect(&addr).map_err(|e| {
        eprintln!("Failed to connect to MCP server at {addr}: {e}");
        eprintln!("Make sure the synth is running with --features mcp");
        e
    })?;

    eprintln!("Connected to MCP server at {addr}");

    let mut reader = stream.try_clone()?;
    let mut writer = stream;

    // stdin → TCP
    let stdin_thread = thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; 8192];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    if writer.flush().is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // TCP → stdout
    let mut stdout = io::stdout().lock();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if stdout.write_all(&buf[..n]).is_err() {
                    break;
                }
                if stdout.flush().is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = stdin_thread.join();
    Ok(())
}
