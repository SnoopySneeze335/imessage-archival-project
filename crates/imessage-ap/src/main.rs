use std::{
    net::SocketAddr,
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
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
    /// Import an iPhone backup folder into the archive. The backup is opened
    /// strictly read-only; running it again adds nothing.
    Ingest {
        /// Backup folder (the one containing Manifest.db).
        #[arg(long)]
        backup: PathBuf,
        /// Data directory; archive.db is created here.
        #[arg(long, env = "IMESSAGE_AP_DATA", default_value = "/data")]
        data: PathBuf,
    },
    /// Copy an ingested backup's attachment files into the data directory's
    /// blob store, once per distinct content, and make their thumbnails.
    /// Resumable: an interrupted run continues where it stopped, and running
    /// it again copies nothing. The backup's files are only read.
    Attachments {
        /// Backup folder (the one containing Manifest.db), already ingested.
        #[arg(long)]
        backup: PathBuf,
        /// Data directory containing archive.db; files go to blobs/ and derived/.
        #[arg(long, env = "IMESSAGE_AP_DATA", default_value = "/data")]
        data: PathBuf,
        /// Thumbnails made at once. Defaults to the CPUs available, which
        /// follows a container's --cpus limit.
        #[arg(long)]
        jobs: Option<usize>,
        /// Stop after reading this many files (to try a run, or test resuming).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Make image previews and audio/video conversions from the files in
    /// the blob store, for what browsers cannot show or play as it is.
    /// Reads only archive.db and blobs/ (no backup needed) and writes to
    /// derived/. Resumable: a stopped run continues where it left off, and
    /// running it again with nothing pending does nothing.
    Derive {
        /// Data directory containing archive.db and blobs/.
        #[arg(long, env = "IMESSAGE_AP_DATA", default_value = "/data")]
        data: PathBuf,
        /// Work on one kind only: image (HEIC and other previews), video or
        /// voice (voice messages). Default: voice, then image, then video.
        #[arg(long, value_enum)]
        kind: Option<DeriveKind>,
        /// Files converted at once. Defaults to the CPUs available, which
        /// follows a container's --cpus limit.
        #[arg(long)]
        jobs: Option<usize>,
        /// Stop after starting this many files.
        #[arg(long)]
        limit: Option<usize>,
        /// Free space to keep on the disk holding derived/, in binary units
        /// as `df -h` shows them (20G = 20 GiB). The run stops cleanly
        /// before a file could take it below this.
        #[arg(long, default_value = "5G", value_parser = imessage_ap::derive::parse_size)]
        min_free: u64,
        /// Only report what a run would do: files per kind and action, with
        /// projected output size and run time. Converts nothing.
        #[arg(long)]
        plan: bool,
    },
    /// Upgrade archive.db to this build's schema, then exit. Ingest does the
    /// same on its own; this only migrates, so stats can be checked first.
    Migrate {
        /// Data directory containing archive.db.
        #[arg(long, env = "IMESSAGE_AP_DATA", default_value = "/data")]
        data: PathBuf,
    },
    /// Print archive counts (no message content) for comparison with the
    /// backup diagnostics in docs/SPEC.md.
    Stats {
        /// Data directory containing archive.db.
        #[arg(long, env = "IMESSAGE_AP_DATA", default_value = "/data")]
        data: PathBuf,
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DeriveKind {
    Image,
    Video,
    Voice,
}

impl DeriveKind {
    fn kind(self) -> imessage_ap::media::convert::Kind {
        use imessage_ap::media::convert::Kind;
        match self {
            Self::Image => Kind::Preview,
            Self::Video => Kind::Video,
            Self::Voice => Kind::Audio,
        }
    }
}

/// The name a derivative kind has on the command line.
fn kind_label(kind: &str) -> &str {
    match kind {
        "preview" => "image",
        "audio" => "voice",
        other => other,
    }
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
        Command::Ingest { backup, data } => {
            let summary = imessage_ap::ingest::run(&backup, &data)?;
            let c = &summary.counts;
            println!(
                "Import run {} (backup {}) succeeded",
                summary.run_id, summary.source_id
            );
            println!(
                "  handles      {:>9} seen {:>9} added",
                c.handles_seen, c.handles_added
            );
            println!(
                "  chats        {:>9} seen {:>9} added",
                c.chats_seen, c.chats_added
            );
            println!(
                "  messages     {:>9} seen {:>9} added",
                c.messages_seen, c.messages_added
            );
            println!(
                "  versions     {:>9} added {:>9} messages changed",
                c.message_versions_added, c.messages_changed
            );
            println!("  edits        {:>9} added", c.message_edits_added);
            println!("  reactions    {:>9} added", c.reactions_added);
            println!(
                "  attachments  {:>9} seen {:>9} added {:>9} with file",
                c.attachments_seen, c.attachments_added, c.attachment_files_present
            );
            println!("  issues       {:>9}", c.issues);
            for (kind, n) in &summary.issues_by_kind {
                println!("    {kind:<32}{n:>9}");
            }
            Ok(())
        }
        Command::Attachments {
            backup,
            data,
            jobs,
            limit,
        } => {
            let opts = imessage_ap::attachments::Options {
                jobs: jobs.unwrap_or_else(imessage_ap::attachments::default_jobs),
                limit,
                stop: Arc::new(AtomicBool::new(false)),
            };
            tokio::spawn(stop_on_signal(opts.stop.clone()));
            let summary = tokio::task::spawn_blocking(move || {
                imessage_ap::attachments::run(&backup, &data, &opts)
            })
            .await??;
            print_attachments(&summary);
            Ok(())
        }
        Command::Derive {
            data,
            kind,
            jobs,
            limit,
            min_free,
            plan,
        } => {
            let opts = imessage_ap::derive::Options {
                jobs: jobs.unwrap_or_else(imessage_ap::attachments::default_jobs),
                kinds: kind.map_or_else(
                    || imessage_ap::media::convert::Kind::ALL.to_vec(),
                    |k| vec![k.kind()],
                ),
                limit,
                min_free,
                stop: Arc::new(AtomicBool::new(false)),
                stop_after: None,
            };
            tokio::spawn(stop_on_signal(opts.stop.clone()));
            if plan {
                let report =
                    tokio::task::spawn_blocking(move || imessage_ap::derive::plan(&data, &opts))
                        .await??;
                print_plan(&report);
                return Ok(());
            }
            let summary =
                tokio::task::spawn_blocking(move || imessage_ap::derive::run(&data, &opts))
                    .await??;
            print_derive(&summary);
            if let Some(l) = &summary.low_disk {
                anyhow::bail!(
                    "stopped to keep free space: {:.1} GiB free on the disk of derived/, {:.1} GiB needed; free some space (or let the mover run) and run again",
                    l.free as f64 / f64::from(1u32 << 30),
                    l.needed as f64 / f64::from(1u32 << 30)
                );
            }
            Ok(())
        }
        Command::Migrate { data } => {
            anyhow::ensure!(
                imessage_ap::archive::path(&data).is_file(),
                "no archive at {}",
                imessage_ap::archive::path(&data).display()
            );
            drop(imessage_ap::archive::open(&data)?);
            println!(
                "archive.db is at schema version {}",
                imessage_ap::archive::schema_version()
            );
            Ok(())
        }
        Command::Stats { data, json } => {
            let stats = imessage_ap::stats::collect(&data)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                print!("{}", imessage_ap::stats::render(&stats));
            }
            Ok(())
        }
    }
}

fn print_attachments(summary: &imessage_ap::attachments::Summary) {
    let c = &summary.counts;
    let gb = |b: u64| format!("{:.2} GB", b as f64 / 1e9);
    let state = if summary.interrupted {
        "was interrupted; run the same command again to resume"
    } else {
        "succeeded"
    };
    println!(
        "Attachment run {} (backup {}) {state}",
        summary.run_id, summary.source_id
    );
    println!(
        "  files to copy at start    {:>9}",
        c.files_pending_at_start
    );
    println!(
        "  files read                {:>9}  {}",
        c.files_read,
        gb(c.bytes_read)
    );
    println!(
        "  new blobs                 {:>9}  {}",
        c.blobs_added,
        gb(c.bytes_stored)
    );
    println!("  duplicates skipped        {:>9}", c.duplicates_skipped);
    println!("  read failures             {:>9}", c.read_failures);
    println!("  thumbnails made           {:>9}", c.thumbnails_made);
    println!("  thumbnails failed         {:>9}", c.thumbnails_failed);
    if c.thumbnails_retried > 0 {
        println!("    (retried, decoder fix)  {:>9}", c.thumbnails_retried);
    }
    println!("  conversions queued        {:>9}", c.derivatives_queued);
    println!("  files still to copy       {:>9}", c.files_pending_at_end);
    println!(
        "  thumbnails still to make  {:>9}",
        c.thumbnails_pending_at_end
    );
    if let Some(why) = &summary.thumbnails_skipped {
        println!("  thumbnails skipped: {why}");
    }
}

fn print_derive(summary: &imessage_ap::derive::Summary) {
    let c = &summary.counts;
    let state = if summary.low_disk.is_some() {
        "stopped for disk space; run the same command again once there is room"
    } else if summary.interrupted {
        "was interrupted; run the same command again to resume"
    } else {
        "succeeded"
    };
    println!("Derive run {} {state}", summary.run_id);
    println!("  files started             {:>9}", c.files_started);
    println!(
        "  {:<8}{:>9}{:>9}{:>9}{:>9}{:>9}{:>12}{:>9}",
        "kind", "pending", "retried", "made", "original", "failed", "MB written", "left"
    );
    for (kind, k) in &c.kinds {
        println!(
            "  {:<8}{:>9}{:>9}{:>9}{:>9}{:>9}{:>12.1}{:>9}",
            kind_label(kind),
            k.pending_at_start,
            k.retried,
            k.made,
            k.original,
            k.failed,
            k.bytes_written as f64 / 1e6,
            k.pending_at_end
        );
    }
}

fn print_plan(report: &imessage_ap::derive::PlanReport) {
    let gb = |b: u64| b as f64 / 1e9;
    let hours = |s: f64| s / 3600.0 / report.jobs as f64;
    println!(
        "Derive plan (nothing converted; projections are rough, for {} jobs)",
        report.jobs
    );
    println!(
        "  {:<7}{:<12}{:>7}{:>10}{:>10}{:>16}{:>14}",
        "kind", "action", "files", "input GB", "length h", "output GB", "time h"
    );
    let (mut out, mut time) = ((0u64, 0u64), (0f64, 0f64));
    for (kind, actions) in &report.rows {
        for (action, r) in actions {
            println!(
                "  {:<7}{:<12}{:>7}{:>10.2}{:>10.1}{:>16}{:>14}",
                kind_label(kind),
                action,
                r.files,
                gb(r.input_bytes),
                r.seconds / 3600.0,
                format!("{:.2}–{:.2}", gb(r.output_bytes.0), gb(r.output_bytes.1)),
                format!(
                    "{:.1}–{:.1}",
                    hours(r.cpu_seconds.0),
                    hours(r.cpu_seconds.1)
                ),
            );
            out = (out.0 + r.output_bytes.0, out.1 + r.output_bytes.1);
            time = (time.0 + r.cpu_seconds.0, time.1 + r.cpu_seconds.1);
        }
    }
    println!(
        "  {:<7}{:<12}{:>7}{:>10}{:>10}{:>16}{:>14}",
        "total",
        "",
        "",
        "",
        "",
        format!("{:.2}–{:.2}", gb(out.0), gb(out.1)),
        format!("{:.1}–{:.1}", hours(time.0), hours(time.1)),
    );
}

/// First Ctrl-C or SIGTERM: finish the current file and stop cleanly.
/// Second: exit at once (the next run still resumes safely).
async fn stop_on_signal(stop: Arc<AtomicBool>) {
    for _ in 0..2 {
        shutdown_signal().await;
        if stop.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        tracing::warn!("stopping after the current file; press Ctrl-C again to quit now");
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
