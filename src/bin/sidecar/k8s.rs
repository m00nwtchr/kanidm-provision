use color_eyre::eyre::Result;
use futures::StreamExt;
use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::api::core::v1::Secret;
use kanidm_provision::run_provisioning;
use kanidm_provision::state::State;
use kube::api::{ObjectMeta, Patch, PatchParams};
use kube::runtime::watcher::Config as WatcherConfig;
use kube::runtime::{watcher, WatchStreamExt};
use kube::{Api, Client};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::sleep;
use tracing::{error, info, warn};
pub async fn get_merged_state(client: &Client, namespace: &str) -> Result<serde_json::Value> {
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    let lp = kube::api::ListParams::default().labels("kanidm_config=1");
    let cms = api.list(&lp).await?;

    let mut merged = serde_json::json!({
      "groups": {},
      "persons": {},
      "systems": {"oauth2": {} }
    });

    for cm in cms {
        for json in extract_json_from_cm(&cm) {
            deep_merge(&mut merged, json);
        }
    }

    Ok(merged)
}

fn extract_json_from_cm(cm: &ConfigMap) -> Vec<serde_json::Value> {
    let Some(data) = &cm.data else {
        return vec![];
    };
    data.iter()
        .filter(|(key, _)| key.ends_with(".json"))
        .filter_map(|(_, value)| serde_json::from_str(value).ok())
        .collect()
}

fn deep_merge(base: &mut serde_json::Value, override_val: serde_json::Value) {
    match (base, override_val) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(override_map)) => {
            for (k, v) in override_map {
                deep_merge(base_map.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (serde_json::Value::Array(base_arr), serde_json::Value::Array(override_arr)) => {
            for v in override_arr {
                if !base_arr.contains(&v) {
                    base_arr.push(v);
                }
            }
        }
        (base_val, override_val) => {
            *base_val = override_val;
        }
    }
}

async fn fetch_basic_secret(
    http_client: &reqwest::Client,
    kanidm_url: &str,
    token: &str,
    client_name: &str,
) -> Result<String> {
    let url = format!("{kanidm_url}/v1/oauth2/{client_name}/_basic_secret");
    let secret = http_client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json::<String>()
        .await?;
    Ok(secret)
}

pub async fn reconcile_secret(
    http_client: &reqwest::Client,
    client: &Client,
    namespace: &str,
    state: &State,
    kanidm_url: &str,
    kanidm_token: &str,
) {
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    for (name, oauth2) in &state.systems.oauth2 {
        let Some(k8s_config) = &oauth2.k8s else {
            continue;
        };
        if oauth2.public {
            continue;
        }
        let secret_val = match fetch_basic_secret(http_client, kanidm_url, kanidm_token, name).await {
            Ok(val) => val,
            Err(e) => {
                error!(client = %name, error = format!("{e:#}"), "Failed to fetch secret");
                continue;
            }
        };
        let secret_name = format!("kanidm-{name}-oidc");
        let mut string_data = BTreeMap::new();
        string_data.insert(k8s_config.client_id_key.clone(), name.clone());
        string_data.insert(k8s_config.client_secret_key.clone(), secret_val);

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(secret_name.clone()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            string_data: Some(string_data),
            ..Default::default()
        };
        if let Err(e) = api
            .patch(
                &secret_name,
                &PatchParams::apply("kanidm-provision"),
                &Patch::Apply(secret),
            )
            .await
        {
            error!(secret = %secret_name, %namespace, error = format!("{e:#}"), "Failed to patch secret");
            continue;
        }
        info!(secret = %secret_name, %namespace, "Reconciled secret");
    }
}

async fn reconcile(
    http_client: &reqwest::Client,
    client: &Client,
    namespace: &str,
    kanidm_url: &str,
    kanidm_token: &str,
    no_auto_remove: bool,
) -> Result<()> {
    let state_val = get_merged_state(client, namespace).await?;
    let mut state: State = serde_json::from_value(state_val)?;
    download_icons(http_client, &mut state, kanidm_url, kanidm_token).await;
    let url = kanidm_url.to_string();
    let token = kanidm_token.to_string();
    let state = tokio::task::spawn_blocking(move || -> color_eyre::eyre::Result<State> {
        run_provisioning(&url, &token, &state, false, no_auto_remove)?;
        Ok(state)
    })
    .await
    .map_err(|e| color_eyre::eyre::eyre!("Task panicked: {e}"))??;
    reconcile_secret(http_client, client, namespace, &state, kanidm_url, kanidm_token).await;
    Ok(())
}

async fn download_single_icon(
    http_client: &reqwest::Client,
    name: &str,
    image_url: &str,
    icons_dir: &std::path::Path,
) -> Result<String> {
    let icon_path = icons_dir.join(format!("{name}.svg"));
    if !icon_path.exists() {
        info!(%name, %image_url, "Downloading icon");
        let bytes = http_client
            .get(image_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        tokio::fs::write(&icon_path, bytes).await?;
    }
    Ok(icon_path.to_string_lossy().into_owned())
}

async fn kanidm_has_image(http_client: &reqwest::Client, kanidm_url: &str, token: &str, name: &str) -> bool {
    let url = format!("{kanidm_url}/ui/images/oauth2/{name}");
    match http_client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

async fn download_icons(http_client: &reqwest::Client, state: &mut State, kanidm_url: &str, token: &str) {
    let icons_dir = std::path::Path::new("/data/icons");
    if let Err(e) = tokio::fs::create_dir_all(icons_dir).await {
        error!(error = format!("{e:#}"), "Failed to create icons directory");
        return;
    }

    for (name, oauth2) in &mut state.systems.oauth2 {
        let Some(image_url) = oauth2.k8s.as_ref().and_then(|k| k.image_url.as_deref()) else {
            continue;
        };
        if kanidm_has_image(http_client, kanidm_url, token, name).await {
            continue;
        }
        match download_single_icon(http_client, name, image_url, icons_dir).await {
            Ok(icon_path) => {
                oauth2.image_file = Some(icon_path);
            }
            Err(e) => {
                warn!(%name, error = format!("{e:#}"), "Failed to download icon");
            }
        }
    }
}

pub async fn wait_for_kanidm(kanidm_url: &str) -> Result<()> {
    let url = format!("{kanidm_url}/status");
    let mut sigterm = signal(SignalKind::terminate())?;
    info!(%url, "Waiting for Kanidm to be ready");
    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM while waiting for Kanidm, shutting down");
                return Err(color_eyre::eyre::eyre!("interrupted by SIGTERM"));
            }
            result = reqwest::get(&url) => {
                match result {
                    Ok(resp) if resp.status().is_success() => {
                        info!("Kanidm is ready");
                        return Ok(());
                    }
                    Ok(resp) => {
                        warn!(status = %resp.status(), "Kanidm not ready, retrying in 2s");
                    }
                    Err(e) => {
                        warn!(error = format!("{e:#}"), "Kanidm health check failed, retrying in 2s");
                    }
                }
            }
        }
        sleep(Duration::from_secs(2)).await;
    }
}

pub async fn watch_and_reconcile(
    client: &Client,
    namespace: &str,
    kanidm_url: &str,
    kanidm_token: &str,
    no_auto_remove: bool,
) -> Result<()> {
    wait_for_kanidm(kanidm_url).await?;
    let http_client = reqwest::Client::new();
    let api = Api::<ConfigMap>::namespaced(client.clone(), namespace);
    let cfg = WatcherConfig::default().labels("kanidm_config=1");

    let mut stream = watcher(api, cfg).default_backoff().boxed();
    let mut sigterm = signal(SignalKind::terminate())?;

    info!(%namespace, "Watching for ConfigMap changes");

    if let Err(e) = reconcile(
        &http_client,
        client,
        namespace,
        kanidm_url,
        kanidm_token,
        no_auto_remove,
    )
    .await
    {
        error!(error = format!("{e:#}"), "Error during initial reconciliation");
    }

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down gracefully");
                break;
            }
            event = stream.next() => {
                match event {
                    Some(Ok(_)) => {
                        info!("Change detected, reconciling");
                        sleep(Duration::from_secs(2)).await;
                        if let Err(e) = reconcile(&http_client, client, namespace, kanidm_url, kanidm_token, no_auto_remove).await {
                            error!(error = format!("{e:#}"), "Error during reconciliation");
                        }
                    }
                    Some(Err(e)) => error!(error = %e, "Watcher error"),
                    None => break,
                }
            }
        }
    }
    Ok(())
}
