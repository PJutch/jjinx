use futures::io;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::{ClientConfig, RootCertStore};
use rustls::{ServerConfig, sign::CertifiedKey};
use rustls_native_certs::load_native_certs;
use std::collections::HashMap;
use std::sync::Arc;
use std::{fs::File, io::BufReader};
use thiserror::Error;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::config::ServerGroup;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("No private keys in the file")]
    NoPrivateKey,
    #[error("Tls error")]
    TlsError(rustls::Error),
}

#[derive(Debug, Default)]
pub struct Resolver {
    by_name: HashMap<String, Arc<CertifiedKey>>,
    default: Option<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = client_hello.server_name() {
            self.by_name
                .get(name)
                .or_else(|| self.default.as_ref())
                .cloned()
        } else {
            self.default.clone()
        }
    }
}

pub fn make_acceptor(servers: &ServerGroup) -> Result<Option<TlsAcceptor>, TlsError> {
    if servers
        .servers
        .iter()
        .any(|server| server.cert_path.is_some())
    {
        let config_builder = ServerConfig::builder().with_no_client_auth();

        let mut cert_resolver = Resolver::default();
        for server in &servers.servers {
            if let Some(cert_path) = &server.cert_path {
                let mut cert_reader =
                    BufReader::new(File::open(cert_path).map_err(TlsError::IoError)?);
                let certs = rustls_pemfile::certs(&mut cert_reader)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(TlsError::IoError)?;

                let mut keys_reader = BufReader::new(
                    File::open(server.keys_path.as_ref().unwrap()).map_err(TlsError::IoError)?,
                );
                let key = rustls_pemfile::private_key(&mut keys_reader)
                    .map_err(TlsError::IoError)?
                    .ok_or(TlsError::NoPrivateKey)?;

                let cert_key = CertifiedKey::from_der(
                    certs.clone(),
                    key.clone_key(),
                    config_builder.crypto_provider(),
                )
                .map_err(TlsError::TlsError)?;

                if !server.domain_names.is_empty() {
                    for domain in &server.domain_names {
                        cert_resolver
                            .by_name
                            .insert(domain.clone(), Arc::new(cert_key.clone()));
                    }
                } else {
                    cert_resolver.default = Some(Arc::new(cert_key))
                }
            }
        }
        let config = config_builder.with_cert_resolver(Arc::new(cert_resolver));

        Ok(Some(TlsAcceptor::from(Arc::new(config))))
    } else {
        Ok(None)
    }
}

pub fn make_connector() -> Result<TlsConnector, rustls::Error> {
    let mut root_cert_store = RootCertStore::empty();
    for cert in load_native_certs().certs {
        root_cert_store.add(cert)?;
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}
