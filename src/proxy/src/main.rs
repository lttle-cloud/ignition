use std::sync::Arc;

use anyhow::Result;
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    ServerConfig,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

fn load_server_config() -> Result<ServerConfig> {
    let certs = CertificateDer::pem_file_iter("./certs/server.cert")?
        .map(|cert| cert.map_err(|e| anyhow::anyhow!(e)))
        .collect::<Result<Vec<_>>>()?;

    let key = PrivateKeyDer::from_pem_file("./certs/server.key")?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(config)
}

async fn forward_bidirectional(
    client_stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    mut backend_stream: TcpStream,
) -> Result<()> {
    let (mut client_reader, mut client_writer) = tokio::io::split(client_stream);
    let (mut backend_reader, mut backend_writer) = backend_stream.split();

    // Copy client -> backend
    let forward_to_backend = tokio::io::copy(&mut client_reader, &mut backend_writer);
    // Copy backend -> client
    let forward_to_client = tokio::io::copy(&mut backend_reader, &mut client_writer);

    tokio::try_join!(forward_to_backend, forward_to_client)?;
    Ok(())
}

const SSL_REQUEST_CODE: u32 = 0x04D2162F; // PostgreSQL SSLRequest code

async fn handle_client(incoming_stream: TcpStream, tls_acceptor: Arc<TlsAcceptor>) -> Result<()> {
    let mut reader = BufReader::new(incoming_stream);

    // Read the first 8 bytes for SSLRequest
    let mut initial_bytes: [u8; 8] = [0u8; 8];
    reader.read_exact(&mut initial_bytes).await?;

    let _length = u32::from_be_bytes([
        initial_bytes[0],
        initial_bytes[1],
        initial_bytes[2],
        initial_bytes[3],
    ]);

    let code = u32::from_be_bytes([
        initial_bytes[4],
        initial_bytes[5],
        initial_bytes[6],
        initial_bytes[7],
    ]);

    if code != SSL_REQUEST_CODE {
        eprintln!(
            "Unexpected SSLRequest code: {:#X}. Expected: {:#X}",
            code, SSL_REQUEST_CODE
        );
        // Optionally, close the connection or handle as per your requirements
        return Ok(());
    }

    // Respond with 'S' to accept SSL
    reader.get_mut().write_all(b"S").await?;

    // Proceed with TLS handshake
    let tls_stream = tls_acceptor.accept(reader.into_inner()).await?;

    // Extract SNI
    let server_conn = tls_stream.get_ref().1;
    let sni_name = match server_conn.server_name() {
        Some(name) => name.to_string(),
        None => {
            eprintln!("No SNI provided by client. Using default route");
            "default".to_string()
        }
    };

    println!("SNI: {}", sni_name);

    // Determine backend address based on SNI
    let backend_addr = match sni_name.as_str() {
        "ps1.local" => "127.0.0.1:5561",
        "ps2.local" => "127.0.0.1:5562",
        // Add more SNI-based routes as needed
        _ => "127.0.0.1:6655", // Default backend
    };

    match TcpStream::connect(backend_addr).await {
        Ok(backend_stream) => {
            if let Err(e) = forward_bidirectional(tls_stream, backend_stream).await {
                eprintln!("Forwarding error: {:?}", e);
            }
        }
        Err(e) => {
            eprintln!("Failed to connect to backend '{}': {:?}", backend_addr, e);
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let tls_config = load_server_config()?;
    let tls_acceptor = Arc::new(TlsAcceptor::from(Arc::new(tls_config)));

    let listener = TcpListener::bind("0.0.0.0:5432").await?;
    println!("Proxy listening on {}", listener.local_addr()?);

    loop {
        let (incoming_stream, addr) = listener.accept().await?;
        println!("New connection from: {}", addr);

        let acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(incoming_stream, acceptor).await {
                eprintln!("Connection handling error: {:?}", e);
            }
        });
    }
}
