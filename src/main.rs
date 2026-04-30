use reqwest::{Certificate, Client};
use rustls::crypto::ring;

#[tokio::main]
async fn main() -> Result<(), tube::TubeError> {
  ring::default_provider()
    .install_default()
    .expect("could not install default crypto provider as ring");

  let config = tube::tube_config::TubeConfig::load()?;

  let certs = webpki_root_certs::TLS_SERVER_ROOT_CERTS
    .iter()
    .map(|cert| Certificate::from_der(cert).expect("Failed to parse internal DER"))
    .collect::<Vec<_>>();

  let client = Client::builder()
    .tls_backend_rustls()
    .tls_certs_only(certs)
    .hickory_dns(true)
    .no_proxy()
    .build()?;

  tube::run(config, client).await
}
