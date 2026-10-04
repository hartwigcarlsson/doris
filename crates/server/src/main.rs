use clap::Parser;
use doris_identity::Auth;
use doris_server::bolagsverket::Bolagsverket;
use doris_server::skatteverket::TaxTables;
use doris_server::{AuthApi, CompanyApi, InvoicingApi, LedgerApi, PayrollApi, assets::WebDist};
use http::HeaderValue;
use std::net::SocketAddr;
use std::process::ExitCode;
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
    /// Bolagsverket API client (värdefulla datamängder). Without it, company
    /// details are entered by hand.
    #[arg(long, env = "DORIS_BOLAGSVERKET_CLIENT_ID")]
    bolagsverket_client_id: Option<String>,
    #[arg(long, env = "DORIS_BOLAGSVERKET_CLIENT_SECRET", hide_env_values = true)]
    bolagsverket_client_secret: Option<String>,
    #[arg(long, env = "DORIS_BOLAGSVERKET_TOKEN_URL", default_value = doris_server::bolagsverket::TOKEN_URL)]
    bolagsverket_token_url: String,
    #[arg(long, env = "DORIS_BOLAGSVERKET_API_URL", default_value = doris_server::bolagsverket::API_URL)]
    bolagsverket_api_url: String,
    /// Skatteverket's open dataset of monthly tax tables.
    #[arg(long, env = "DORIS_TAX_TABLES_URL", default_value = doris_server::skatteverket::TAX_TABLES_URL)]
    tax_tables_url: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();
    match run(Config::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("doris: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Starts the server; errors are one sentence for the operator.
async fn run(config: Config) -> Result<(), String> {
    let pool = doris_eventstore::open(&config.database)
        .await
        .map_err(|e| format!("cannot open database {}: {e}", config.database))?;
    let auth = Auth::new(pool.clone(), &config.rp_id, &config.rp_origin)
        .await
        .map_err(|e| format!("cannot set up WebAuthn for {}: {e}", config.rp_origin))?;
    let cors_origins = config
        .cors_origins
        .into_iter()
        .filter(|o| !o.is_empty())
        .collect();
    let bolagsverket = match (
        config.bolagsverket_client_id,
        config.bolagsverket_client_secret,
    ) {
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
            Some(Bolagsverket::new(
                &config.bolagsverket_token_url,
                &config.bolagsverket_api_url,
                id,
                secret,
            ))
        }
        _ => {
            tracing::info!("Bolagsverket lookup off: DORIS_BOLAGSVERKET_CLIENT_ID/_SECRET not set");
            None
        }
    };
    let payroll = PayrollApi::new(pool.clone(), TaxTables::new(&config.tax_tables_url));
    let app = doris_server::router::<WebDist>(
        AuthApi::new(pool.clone(), auth),
        CompanyApi::new(pool.clone(), bolagsverket),
        LedgerApi::new(pool.clone()),
        payroll,
        InvoicingApi::new(pool),
        cors_origins,
        config.serve_frontend,
    );
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", config.listen))?;
    tracing::info!("listening on http://{}", config.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("server stopped: {e}"))
}

/// Resolves on Ctrl-C or, on Unix, SIGTERM (sent by `kill`, container
/// runtimes and orchestrators when stopping the process).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
