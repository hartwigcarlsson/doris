//! Test harness: a real server on an ephemeral port, called over gRPC-Web
//! (HTTP/1.1) exactly like the browser does.

#![allow(dead_code)]

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use doris_identity::Auth;
use doris_proto::auth::v1 as pb;
use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use doris_proto::company::v1::company_service_client::CompanyServiceClient;
use doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient;
use doris_proto::payroll::v1::payroll_service_client::PayrollServiceClient;
use doris_server::bolagsverket::Bolagsverket;
use doris_server::{AuthApi, CompanyApi, LedgerApi, PayrollApi, SESSION_COOKIE};
use http::HeaderValue;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rust_embed::RustEmbed;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tonic::Request;
use tonic_web::{GrpcWebCall, GrpcWebClientLayer, GrpcWebClientService};
use url::Url;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse};

type Transport = GrpcWebClientService<Client<HttpConnector, GrpcWebCall<tonic::body::Body>>>;
pub type Grpc = AuthServiceClient<Transport>;
pub type Companies = CompanyServiceClient<Transport>;
pub type Ledger = LedgerServiceClient<Transport>;
pub type Payroll = PayrollServiceClient<Transport>;
pub type Device = WebauthnAuthenticator<SoftPasskey>;

#[derive(RustEmbed)]
#[folder = "tests/fixtures/dist"]
pub struct TestDist;

pub struct TestServer {
    pub base: String,
    pub origin: Url,
    pub pool: SqlitePool,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with(vec![], true).await
    }

    pub async fn start_with(cors_origins: Vec<HeaderValue>, serve_frontend: bool) -> Self {
        Self::launch(cors_origins, serve_frontend, None).await
    }

    pub async fn start_with_bolagsverket(bolagsverket: Bolagsverket) -> Self {
        Self::launch(vec![], true, Some(bolagsverket)).await
    }

    async fn launch(
        cors_origins: Vec<HeaderValue>,
        serve_frontend: bool,
        bolagsverket: Option<Bolagsverket>,
    ) -> Self {
        let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = Url::parse(&format!("http://localhost:{}", addr.port())).unwrap();
        let auth = Auth::new(pool.clone(), "localhost", &origin).await.unwrap();
        let app = doris_server::router::<TestDist>(
            AuthApi::new(pool.clone(), auth),
            CompanyApi::new(pool.clone(), bolagsverket),
            LedgerApi::new(pool.clone()),
            PayrollApi::new(pool.clone()),
            cors_origins,
            serve_frontend,
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base: format!("http://{addr}"),
            origin,
            pool,
        }
    }

    fn transport(&self) -> Transport {
        let client = Client::builder(TokioExecutor::new()).build_http();
        tower::ServiceBuilder::new()
            .layer(GrpcWebClientLayer::new())
            .service(client)
    }

    pub fn grpc(&self) -> Grpc {
        AuthServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
    }

    pub fn payroll(&self) -> Payroll {
        PayrollServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
    }

    pub fn ledger(&self) -> Ledger {
        // Room for a 10 MiB underlag coming back from GetAttachment.
        LedgerServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
            .max_decoding_message_size(11 << 20)
    }

    pub fn companies(&self) -> Companies {
        CompanyServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
    }

    /// An admin invites `email`, who registers; returns the new user's session.
    pub async fn invite(&self, admin: &str, email: &str) -> String {
        let invite = self
            .grpc()
            .create_invitation(authed(
                pb::CreateInvitationRequest {
                    email: email.into(),
                },
                admin,
            ))
            .await
            .unwrap()
            .into_inner();
        self.sign_up(&mut device(), email, Some(&invite.token))
            .await
    }

    /// Registers through the API and returns the session cookie's token.
    pub async fn sign_up(&self, device: &mut Device, email: &str, token: Option<&str>) -> String {
        let mut grpc = self.grpc();
        let begin = grpc
            .begin_registration(pb::BeginRegistrationRequest {
                email: email.into(),
                display_name: "Anna".into(),
                invitation_token: token.map(Into::into),
                passkey_name: "Laptop".into(),
            })
            .await
            .unwrap()
            .into_inner();
        let options: CreationChallengeResponse = serde_json::from_str(&begin.options_json).unwrap();
        let credential = device
            .do_registration(self.origin.clone(), options)
            .unwrap();
        let response = grpc
            .finish_registration(pb::FinishRegistrationRequest {
                ceremony_id: begin.ceremony_id,
                invitation_token: token.map(Into::into),
                credential_json: serde_json::to_string(&credential).unwrap(),
            })
            .await
            .unwrap();
        session_from(response.metadata()).unwrap()
    }

    pub async fn log_in(&self, device: &mut Device, email: &str) -> Result<String, tonic::Status> {
        let mut grpc = self.grpc();
        let begin = grpc
            .begin_login(pb::BeginLoginRequest {
                email: email.into(),
            })
            .await?
            .into_inner();
        let options: RequestChallengeResponse = serde_json::from_str(&begin.options_json).unwrap();
        let credential = device
            .do_authentication(self.origin.clone(), options)
            .unwrap();
        let response = grpc
            .finish_login(pb::FinishLoginRequest {
                ceremony_id: begin.ceremony_id,
                credential_json: serde_json::to_string(&credential).unwrap(),
            })
            .await?;
        Ok(session_from(response.metadata()).unwrap())
    }
}

pub fn device() -> Device {
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

/// A request carrying the session cookie, as the browser would send it.
pub fn authed<T>(message: T, session: &str) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "cookie",
        format!("theme=dark; {SESSION_COOKIE}={session}")
            .parse()
            .unwrap(),
    );
    request
}

pub fn set_cookie(metadata: &tonic::metadata::MetadataMap) -> Option<String> {
    metadata
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().to_owned())
}

pub fn session_from(metadata: &tonic::metadata::MetadataMap) -> Option<String> {
    let cookie = set_cookie(metadata)?;
    let value = cookie
        .split(';')
        .next()?
        .strip_prefix(&format!("{SESSION_COOKIE}="))?;
    (!value.is_empty()).then(|| value.to_owned())
}

/// A plain HTTP/1.1 request (for static files and CORS preflights).
pub async fn http(
    method: http::Method,
    url: &str,
    headers: &[(&str, &str)],
) -> http::Response<String> {
    http_with_body(method, url, headers, b"").await
}

pub async fn http_with_body(
    method: http::Method,
    url: &str,
    headers: &[(&str, &str)],
    body: &'static [u8],
) -> http::Response<String> {
    use http_body_util::{BodyExt, Full};
    let client = Client::builder(TokioExecutor::new()).build_http::<Full<axum::body::Bytes>>();
    let mut request = http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = client
        .request(
            request
                .body(Full::new(axum::body::Bytes::from_static(body)))
                .unwrap(),
        )
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    http::Response::from_parts(parts, String::from_utf8_lossy(&bytes).into_owned())
}

/// A stand-in for Skatteverket's rowstore dataset, serving `rows` page by
/// page like the real one (`år`, `_limit`, `_offset`).
pub struct FakeSkatteverket {
    pub url: String,
    pub requests: Arc<AtomicUsize>,
    /// Answer 500.
    pub broken: Arc<AtomicBool>,
    /// Promise every row but stop sending after the first page.
    pub truncated: Arc<AtomicBool>,
}

pub async fn fake_skatteverket(rows: Vec<Value>) -> FakeSkatteverket {
    let requests = Arc::new(AtomicUsize::new(0));
    let broken = Arc::new(AtomicBool::new(false));
    let truncated = Arc::new(AtomicBool::new(false));
    let (count, fail, cut) = (requests.clone(), broken.clone(), truncated.clone());
    let app = axum::Router::new().route(
        "/rowstore",
        get(move |Query(q): Query<HashMap<String, String>>| {
            let (rows, count, fail, cut) = (rows.clone(), count.clone(), fail.clone(), cut.clone());
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                if fail.load(Ordering::SeqCst) {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                let year = q.get("år").cloned().unwrap_or_default();
                let limit: usize = q.get("_limit").and_then(|v| v.parse().ok()).unwrap_or(100);
                let offset: usize = q.get("_offset").and_then(|v| v.parse().ok()).unwrap_or(0);
                let matching: Vec<&Value> =
                    rows.iter().filter(|r| r["år"] == year.as_str()).collect();
                let page: Vec<&Value> = if cut.load(Ordering::SeqCst) && offset > 0 {
                    vec![]
                } else {
                    matching.iter().skip(offset).take(limit).copied().collect()
                };
                axum::Json(json!({
                    "resultCount": matching.len(),
                    "offset": offset,
                    "limit": limit,
                    "results": page,
                }))
                .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/rowstore", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    FakeSkatteverket {
        url,
        requests,
        broken,
        truncated,
    }
}

/// A complete year in Skatteverket's shape: tables 29–42, each with amount
/// bands 1–2 000 kr and then 1 000 kr wide up to 80 000 kr, and percent
/// bands 80 001–1 269 000 and 1 269 001 up. Values are synthetic except
/// tabell 33, kolumn 1 on 34 001–35 000 kr: 7 134 kr, as in 2026.
pub fn tax_rows(year: i16) -> Vec<Value> {
    let row = |table: u8, kind: &str, from: i64, to: Option<i64>, cols: [i64; 6]| {
        let mut r = json!({
            "år": year.to_string(),
            "tabellnr": table.to_string(),
            "antal dgr": kind,
            "inkomst fr.o.m.": from.to_string(),
            "inkomst t.o.m.": to.map(|t| t.to_string()).unwrap_or_default(),
            "kolumn 7": "",
        });
        for (i, c) in cols.iter().enumerate() {
            r[format!("kolumn {}", i + 1)] = json!(c.to_string());
        }
        r
    };
    let mut rows = Vec::new();
    for table in 29..=42u8 {
        let mut bands = vec![(1, 2000)];
        bands.extend((2001..80000).step_by(1000).map(|f| (f, f + 999)));
        for (from, to) in bands {
            let mut cols = [1, 2, 3, 4, 5, 6].map(|k| from / 5 + k);
            if table == 33 && from == 34001 {
                cols[0] = 7134;
            }
            rows.push(row(table, "30B", from, Some(to), cols));
        }
        rows.push(row(table, "30%", 80001, Some(1269000), [33; 6]));
        rows.push(row(table, "30%", 1269001, None, [52; 6]));
    }
    rows
}
