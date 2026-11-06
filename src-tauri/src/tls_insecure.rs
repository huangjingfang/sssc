//! insecure_tls 支持：跳过服务端证书校验的 HTTP 连接器。
//!
//! 私有部署普遍使用自签证书 / 内部 CA 的 HTTPS endpoint；当用户在
//! Profile 中显式勾选「忽略证书校验」时，使用本模块的连接器。
//! 实现方式：rustls ClientConfig 挂载一个无条件通过校验的
//! `ServerCertVerifier`（等价 `danger_accept_invalid_certs(true)` 语义），
//! 经 hyper-rustls + hyper-util legacy client 包装为 SDK HttpConnector。

use aws_smithy_runtime_api::client::http::{
    http_client_fn, HttpConnector, HttpConnectorFuture, SharedHttpConnector,
};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_types::body::SdkBody;
use std::fmt;

/// 构造跳过证书校验的 http client（挂到 config Builder 的 `.http_client()`）。
pub fn insecure_http_client() -> aws_sdk_s3::config::SharedHttpClient {
    http_client_fn(|_settings, _components| {
        SharedHttpConnector::new(InsecureConnector::new())
    })
}

type HyperClient = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    BoxBody,
>;

/// hyper legacy client 只要求 body 错误类型实现 `Into<Box<dyn Error>>`，
/// 统一用 io::Error 承载上游 body 错误。
type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, std::io::Error>;

#[derive(Clone)]
struct InsecureConnector {
    client: HyperClient,
}

impl fmt::Debug for InsecureConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InsecureConnector").finish_non_exhaustive()
    }
}

impl InsecureConnector {
    fn new() -> Self {
        let crypto = rustls::crypto::ring::default_provider();
        let config = rustls::ClientConfig::builder_with_provider(crypto.into())
            .with_protocol_versions(rustls::DEFAULT_VERSIONS)
            .expect("默认 TLS 协议版本合法")
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(NoVerify::new()))
            .with_no_client_auth();
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(config)
            .https_or_http()
            .enable_http1()
            .build();
        let client = hyper_util::client::legacy::Client::builder(
            hyper_util::rt::TokioExecutor::new(),
        )
        .build(https);
        Self { client }
    }
}

/// 无条件通过服务端证书校验（等价 danger_accept_invalid_certs(true)）。
#[derive(Debug)]
struct NoVerify {
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl NoVerify {
    fn new() -> Self {
        Self {
            algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

impl HttpConnector for InsecureConnector {
    fn call(&self, request: aws_smithy_runtime_api::http::Request<SdkBody>) -> HttpConnectorFuture {
        let client = self.client.clone();
        HttpConnectorFuture::new(async move {
            use http_body_util::BodyExt;
            // SDK 请求 → hyper 请求（body 错误统一映射为 io::Error）
            let (parts, body) = request
                .try_into_http1x()
                .map_err(|e| ConnectorError::user(Box::new(e)))?
                .into_parts();
            let body = BodyExt::map_err(body, |e| std::io::Error::other(e.to_string()));
            let request = hyper::http::Request::from_parts(parts, BoxBody::new(body));
            let resp = client
                .request(request)
                .await
                .map_err(|e| ConnectorError::io(Box::new(e)))?;
            // hyper 响应 → SDK 响应
            let (parts, body) = resp.into_parts();
            let mut resp = aws_smithy_runtime_api::http::Response::new(
                aws_smithy_runtime_api::http::StatusCode::from(parts.status),
                SdkBody::from_body_1_x(body),
            );
            let pairs: Vec<(String, String)> = parts
                .headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
                .collect();
            let headers = resp.headers_mut();
            for (k, v) in pairs {
                headers.append(k, v);
            }
            Ok(resp)
        })
    }
}
