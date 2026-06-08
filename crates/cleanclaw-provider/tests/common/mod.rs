//! Live provider integration tests using a tiny in-process HTTP
//! server to capture requests and return canned responses.
//!
//! Mirrors the wiremock pattern from the Go provider tests.

#![allow(dead_code)]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

#[derive(Clone)]
pub struct MockBackend {
    pub last_body: Arc<Mutex<Option<Bytes>>>,
    pub last_path: Arc<Mutex<Option<String>>>,
    pub last_auth: Arc<Mutex<Option<String>>>,
    pub response_status: Arc<Mutex<u16>>,
    pub response_body: Arc<Mutex<Bytes>>,
    pub response_headers: Arc<Mutex<Vec<(String, String)>>>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            last_body: Arc::new(Mutex::new(None)),
            last_path: Arc::new(Mutex::new(None)),
            last_auth: Arc::new(Mutex::new(None)),
            response_status: Arc::new(Mutex::new(200)),
            response_body: Arc::new(Mutex::new(Bytes::from_static(b"{}"))),
            response_headers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn serve(&self) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let me2 = me.clone();
                tokio::spawn(async move {
                    let _ = http1::Builder::new()
                        .serve_connection(
                            io,
                            service_fn(move |req: Request<Incoming>| {
                                let me = me2.clone();
                                async move { handle(req, me).await }
                            }),
                        )
                        .await;
                });
            }
        });
        addr
    }

    pub fn set_response(&self, status: u16, body: &[u8]) {
        *self.response_status.lock().unwrap() = status;
        *self.response_body.lock().unwrap() = Bytes::copy_from_slice(body);
    }

    pub fn set_streaming_response(&self, chunks: &[&[u8]]) {
        // For streaming tests, we just dump a single concatenated body;
        // the chunked-SSE parsing is tested separately.
        let combined: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
        *self.response_status.lock().unwrap() = 200;
        *self.response_body.lock().unwrap() = Bytes::from(combined);
    }

    pub fn last_body(&self) -> Option<Bytes> {
        self.last_body.lock().unwrap().clone()
    }

    pub fn last_path(&self) -> Option<String> {
        self.last_path.lock().unwrap().clone()
    }

    pub fn last_auth(&self) -> Option<String> {
        self.last_auth.lock().unwrap().clone()
    }
}

use std::sync::Mutex;

async fn handle(
    req: Request<Incoming>,
    mock: MockBackend,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    *mock.last_path.lock().unwrap() = Some(path.clone());

    if let Some(auth) = req.headers().get("authorization") {
        *mock.last_auth.lock().unwrap() = Some(auth.to_str().unwrap_or("").to_string());
    } else if let Some(auth) = req.headers().get("x-api-key") {
        *mock.last_auth.lock().unwrap() = Some(auth.to_str().unwrap_or("").to_string());
    }

    let body = http_body_util::BodyExt::collect(req.into_body())
        .await
        .unwrap()
        .to_bytes();
    *mock.last_body.lock().unwrap() = Some(body);

    let status = *mock.response_status.lock().unwrap();
    let resp_body = mock.response_body.lock().unwrap().clone();

    let mut resp = Response::builder().status(StatusCode::from_u16(status).unwrap());
    if method == Method::POST {
        resp = resp.header("content-type", "application/json");
    }
    Ok(resp.body(Full::new(resp_body)).unwrap())
}
