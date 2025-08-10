use anyhow::Result;
use tokio::net::TcpStream;

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
