use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use bytes::BytesMut;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    net::TcpStream,
    time::timeout,
};

const MAX_HEADER_BYTES: usize = 16 * 1024; // 16 KiB is NGINXʼs default
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);

// ProtocolExtensions
const PG_SSL_REQUEST_CODE: [u8; 4] = [0x04, 0xD2, 0x16, 0x2F]; // PostgreSQL SSLRequest code

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SniffedProtocol {
    Http,
    Tls,
    PgSsl,
    Unknown,
}

pub async fn sniff_protocol(stream: &TcpStream) -> Result<SniffedProtocol> {
    let mut buf = [0u8; 8];
    let n = stream.peek(&mut buf).await?;
    if n == 0 {
        return Ok(SniffedProtocol::Unknown);
    }

    if n == 8 && buf[4..8] == PG_SSL_REQUEST_CODE {
        return Ok(SniffedProtocol::PgSsl);
    }

    // TLS 1.0-1.3 all start with 0x16 (= Handshake) and a version ≥ 0x0301.
    if buf[0] == 0x16 && buf[1] == 0x03 && buf[2] >= 0x01 {
        return Ok(SniffedProtocol::Tls);
    }

    // Most plain-text HTTP requests start with an ASCII method:
    //   "GET ", "POST ", "HEAD ", "PUT ", "DELETE ", "OPTIONS ", "CONNECT ", "TRACE "
    if buf[0].is_ascii_uppercase() {
        return Ok(SniffedProtocol::Http);
    }

    Ok(SniffedProtocol::Unknown)
}

pub async fn extract_http_host<TConn: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut TConn,
) -> Result<(String, Vec<u8>, Vec<u8>)> {
    async fn read_http_head<TConn: AsyncRead + AsyncWrite + Unpin>(
        stream: &mut TConn,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut buf = BytesMut::with_capacity(1024);
        let start = Instant::now();

        loop {
            // Make sure a slow-loris can’t stall us forever
            if start.elapsed() > HEADER_TIMEOUT || buf.len() >= MAX_HEADER_BYTES {
                bail!("HTTP header timeout");
            }

            // Search for “\r\n\r\n” – end of the head section
            if let Some(pos) = twoway::find_bytes(&buf, b"\r\n\r\n") {
                return Ok((buf[..pos + 4].to_vec(), buf[pos + 4..].to_vec()));
            }

            let n = timeout(HEADER_TIMEOUT, stream.read_buf(&mut buf))
                .await
                .map_err(|_| anyhow::anyhow!("HTTP header read timeout"))??;
            if n == 0 {
                bail!("HTTP header EOF");
            }
        }
    }

    let (head, rest) = read_http_head(stream).await?;

    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);

    match req.parse(&head)? {
        httparse::Status::Complete(_) => {}
        httparse::Status::Partial => bail!("HTTP header partial"),
    };

    if let Some(h) = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("host"))
    {
        let value = std::str::from_utf8(h.value)?.trim();
        return Ok((value.to_owned(), head, rest));
    }

    // 2) CONNECT method: “CONNECT host:port HTTP/1.1”
    if req.method == Some("CONNECT") {
        return Ok((
            req.path
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string(),
            head,
            rest,
        ));
    }

    bail!("HTTP header missing host")
}
