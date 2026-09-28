use clap::Parser;
use doris_identity::Auth;
use doris_server::{AuthApi, assets::WebDist};
use http::HeaderValue;
use std::net::SocketAddr;
use url::Url;

/// Doris bookkeeping server.
#[derive(Parser)]
struct Config {
    /// SQLite database URL.
    #[arg(long, env = "DORIS_DATABASE", default_value = "sqlite://doris.db")]
    database: String,
    #[arg(long, env = "DORIS_LISTEN", default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
    /// WebAuthn relying party id: the domain users see, without scheme or port.
    #[arg(long, env = "DORIS_RP_ID", default_value = "localhost")]
    rp_id: String,
    /// Origin the browser loads the frontend from.
    #[arg(long, env = "DORIS_RP_ORIGIN", default_value = "http://localhost:3000")]
    rp_origin: Url,
    /// Other origins allowed to call the API (frontend on a CDN), comma separated.
    #[arg(long, env = "DORIS_CORS_ORIGINS", value_delimiter = ',')]
    cors_origins: Vec<HeaderValue>,
    /// Serve the embedded frontend.
    #[arg(long, env = "DORIS_SERVE_FRONTEND", default_value_t = true, action = clap::ArgAction::Set)]
    serve_frontend: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let config = Config::parse();
    let pool = doris_eventstore::open(&config.database).await?;
    let auth = Auth::new(pool.clone(), &config.rp_id, &config.rp_origin).await?;
    let app = doris_server::router::<WebDist>(
        AuthApi::new(pool, auth),
        config.cors_origins,
        config.serve_frontend,
    );
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("listening on http://{}", config.listen);
    axum::serve(listener, app).await?;
    Ok(())
}
