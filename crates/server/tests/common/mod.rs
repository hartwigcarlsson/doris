//! Test harness: a real server on an ephemeral port, called over gRPC-Web
//! (HTTP/1.1) exactly like the browser does.

#![allow(dead_code)]

use doris_identity::Auth;
use doris_proto::auth::v1 as pb;
use doris_proto::auth::v1::auth_service_client::AuthServiceClient;
use doris_proto::company::v1::company_service_client::CompanyServiceClient;
use doris_proto::invoicing::v1::invoicing_service_client::InvoicingServiceClient;
use doris_proto::ledger::v1::ledger_service_client::LedgerServiceClient;
use doris_server::bolagsverket::Bolagsverket;
use doris_server::{AuthApi, CompanyApi, InvoicingApi, LedgerApi, SESSION_COOKIE};
use http::HeaderValue;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rust_embed::RustEmbed;
use sqlx::SqlitePool;
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
pub type Invoicing = InvoicingServiceClient<Transport>;
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
            InvoicingApi::new(pool.clone()),
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

    pub fn ledger(&self) -> Ledger {
        // Room for a 10 MiB underlag coming back from GetAttachment.
        LedgerServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
            .max_decoding_message_size(11 << 20)
    }

    pub fn invoicing(&self) -> Invoicing {
        // Room for a 10 MiB underlag coming back from GetSupplierInvoiceAttachment.
        InvoicingServiceClient::with_origin(self.transport(), self.base.parse().unwrap())
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
