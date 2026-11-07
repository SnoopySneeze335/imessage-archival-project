use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

use anyhow::Context;
use clap::{Parser, Subcommand};
use imessage_ap::server;
use tracing_subscriber::EnvFilter;

/// Self-hosted archive and viewer for iMessage history.
#[derive(Debug, Parser)]
#[command(name = "imessage-ap", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the web server.
    Serve {
        /// Address to listen on.
        #[arg(long, env = "IMESSAGE_AP_BIND", default_value = "0.0.0.0")]
        bind: std::net::IpAddr,
        /// Port to listen on.
        #[arg(long, env = "IMESSAGE_AP_PORT", default_value_t = 8080)]
        port: u16,
        /// Data directory (archive.db, user.db, blobs, derived files).
        #[arg(long, env = "IMESSAGE_AP_DATA", default_value = "/data")]
        data: PathBuf,
        /// Directory with the built web app.
        #[arg(long, env = "IMESSAGE_AP_WEB", default_value = "/app/web")]
        web: PathBuf,
    },
    /// Exit 0 if the local server answers /api/health (used by Docker).
    Healthcheck {
        /// Port the server listens on.
        #[arg(long, env = "IMESSAGE_AP_PORT", default_value_t = 8080)]
        port: u16,
    },
    /// Import an iPhone backup folder into the archive (Milestone 2).
    Ingest {
        /// Path to the backup folder (the one containing Manifest.db).
        backup: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Serve {
            bind,
            port,
            data,
            web,
        } => {
            std::fs::create_dir_all(&data)
                .with_context(|| format!("creating data directory {}", data.display()))?;
            let app = server::router(&server::Config { web_dir: web });
            let addr = SocketAddr::new(bind, port);
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("binding {addr}"))?;
            tracing::info!(%addr, version = imessage_ap::VERSION, "listening");
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await?;
            Ok(())
        }
        Command::Healthcheck { port } => healthcheck(port),
        Command::Ingest { backup } => {
            anyhow::bail!(
                "ingest is not implemented yet (Milestone 2); backup: {}",
                backup.display()
            )
        }
    }
}

/// Minimal HTTP/1.0 GET against localhost, so the image needs no curl.
fn healthcheck(port: u16) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::time::Duration;

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status_ok = response
        .lines()
        .next()
        .is_some_and(|line| line.split_whitespace().nth(1) == Some("200"));
    anyhow::ensure!(status_ok, "health check failed");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
