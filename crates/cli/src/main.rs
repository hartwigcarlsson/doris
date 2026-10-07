#[tokio::main]
async fn main() {
    let env = doris_cli::Env::from_process();
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    let code = doris_cli::run(std::env::args_os(), &env, &mut out, &mut err).await;
    std::process::exit(code);
}
