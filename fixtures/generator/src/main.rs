use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

/// Generate a fake iPhone backup with invented people, for tests.
#[derive(Debug, Parser)]
#[command(name = "make-fixture", version)]
struct Args {
    /// Output folder (created; must be empty or missing).
    #[arg(long, default_value = "fixtures/out/backup-v1")]
    out: PathBuf,
    /// Delete the output folder first if it exists.
    #[arg(long)]
    force: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.force && args.out.exists() {
        std::fs::remove_dir_all(&args.out)?;
    }
    let expected = fixture_gen::generate(&args.out)?;
    println!(
        "wrote {}: {} messages, {} chats, {} handles, {} attachments ({} missing)",
        args.out.display(),
        expected.messages,
        expected.chats,
        expected.handles,
        expected.attachments,
        expected.attachments_missing_file + expected.attachments_without_path,
    );
    Ok(())
}
