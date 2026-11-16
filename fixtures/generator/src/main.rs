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
    /// Which backup to write: v1 (first), v2 (a later backup of the same
    /// phone) or demo (v1 plus the "All message kinds" conversations).
    #[arg(long, default_value = "v1", value_parser = ["v1", "v2", "demo"])]
    variant: String,
    /// With --variant demo: add a conversation of 25,000 messages.
    #[arg(long)]
    large: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.force && args.out.exists() {
        std::fs::remove_dir_all(&args.out)?;
    }
    let variant = match args.variant.as_str() {
        "v2" => fixture_gen::Variant::V2,
        "demo" => fixture_gen::Variant::Demo { large: args.large },
        _ => fixture_gen::Variant::V1,
    };
    anyhow::ensure!(
        !args.large || matches!(variant, fixture_gen::Variant::Demo { .. }),
        "--large needs --variant demo"
    );
    let expected = fixture_gen::generate_variant(&args.out, variant)?;
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
