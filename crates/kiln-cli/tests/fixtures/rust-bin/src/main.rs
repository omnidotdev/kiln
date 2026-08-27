// A std-only Rust HTTP server. Exercises copying the compiled binary out of the
// `/app/target` cache mount (a cache mount is not in the image layer) and the
// exec-form CMD on the debian-slim runtime.
use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).unwrap();
    println!("listening on 0.0.0.0:{port}");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nok\n");
    }
}
