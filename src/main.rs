use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use kanidm_provision::{run_provisioning, state::State};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// The URL of the kanidm instance
    #[arg(long)]
    url: String,

    /// A JSON file describing the desired target state. Refer to the README for a description of
    /// the required schema.
    #[arg(long)]
    state: PathBuf,

    /// DANGEROUS! Accept invalid TLS certificates, e.g. for testing instances.
    #[arg(long)]
    accept_invalid_certs: bool,

    /// Do not automatically remove orphaned entities that were previously provisioned
    /// but have since been removed from the state file.
    #[arg(long)]
    no_auto_remove: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = Cli::parse();
    let token = std::env::var("KANIDM_TOKEN").map_err(|_| eyre!("KANIDM_TOKEN environment variable not set"))?;

    let state: State = serde_json::from_str(&std::fs::read_to_string(&args.state)?)?;
    run_provisioning(
        &args.url,
        &token,
        &state,
        args.accept_invalid_certs,
        args.no_auto_remove,
    )?;

    Ok(())
}
