mod k8s;

use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use kube::Client;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    version,
    about = "Kubernetes sidecar that watches ConfigMaps and reconciles kanidm state"
)]
struct Cli {
    /// The URL of the kanidm instance
    #[arg(long, env = "KANIDM_INSTANCE")]
    url: String,

    /// Namespace to watch for changes
    #[arg(long, env = "KANIDM_NAMESPACE")]
    namespace: String,

    /// Automatically remove orphaned entities that were previously provisioned but are
    /// no longer declared in any ConfigMap. Off by default (matches the pre-sidecar bash
    /// behavior which always passed --no-auto-remove); enabling it will delete a group
    /// referenced only by scope_maps/claim_maps, which then makes those maps 404.
    #[arg(long)]
    auto_remove: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let args = Cli::parse();
    let token = std::env::var("KANIDM_TOKEN").map_err(|_| eyre!("KANIDM_TOKEN environment variable not set"))?;

    let client = Client::try_default().await?;
    k8s::watch_and_reconcile(&client, &args.namespace, &args.url, &token, !args.auto_remove).await
}
