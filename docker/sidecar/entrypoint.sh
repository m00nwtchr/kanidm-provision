#!/usr/bin/env bash

set -euo pipefail

LABEL_SELECTOR="kanidm_config=1"
NAMESPACE="${KANIDM_NAMESPACE:-kanidm}"
BASE='{"groups": {}, "persons": {}, "systems": {"oauth2": {}}}'

SA_DIR="/var/run/secrets/kubernetes.io/serviceaccount"

DEBOUNCE_SECONDS="2"

KANIDM_TOKEN_FILE="${KANIDM_TOKEN_FILE:="$PWD/token"}"
KANIDM_TOKEN="${KANIDM_TOKEN:="$(cat "$KANIDM_TOKEN_FILE")"}"

KANIDM_INSTANCE="${KANIDM_INSTANCE:="https://idm.m00nlit.dev"}"

[ -f "$SA_DIR/token" ] && [ -f "$SA_DIR/ca.crt" ]
isSA=$?

if ((isSA == 0)); then
  host="${KUBERNETES_SERVICE_HOST:?KUBERNETES_SERVICE_HOST not set}"
  port="${KUBERNETES_SERVICE_PORT:-443}"

  # If it's an IPv6 literal (contains ':'), wrap in [ ]; if already bracketed, leave it.
  if [[ "$host" == *:* ]]; then
    if [[ "$host" != \[*\] ]]; then
      host="[$host]"
    fi
  fi

  cluster="https://${host}:${port}"

  token="$(cat "$SA_DIR/token")"
  serviceNS="$(cat "$SA_DIR/namespace")"
fi

log() {
  printf '[%s] %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" "$*" >&2
}

use_incluster_sa() {
  if ((isSA == 0)); then
    local tmp_config
    tmp_config="$(mktemp)"
    export KUBECONFIG="$tmp_config"
    trap 'rm -f "$KUBECONFIG"' EXIT

    kubectl config set-cluster in-cluster \
      --server="$cluster" \
      --certificate-authority="$SA_DIR/ca.crt" \
      --embed-certs=true >/dev/null

    kubectl config set-credentials in-cluster-user \
      --token="$token" >/dev/null

    kubectl config set-context in-cluster \
      --cluster=in-cluster \
      --user=in-cluster-user \
      --namespace="$serviceNS" >/dev/null

    kubectl config use-context in-cluster >/dev/null
    log "Using in-cluster serviceaccount (namespace: $serviceNS)"
  else
    log "No in-cluster serviceaccount found, using default kubeconfig"
  fi
}

k8s_patch_secret() {
  ns="$1"
  name="$2"
  payload="$3"

  if ((isSA == 0)); then
    ca="$SA_DIR/ca.crt"

    http_code="$(curl -sS -o /dev/null -w '%{http_code}' \
      --cacert "$ca" \
      -H "Authorization: Bearer $token" \
      -H 'Content-Type: application/merge-patch+json' \
      -X PATCH "${cluster}/api/v1/namespaces/${ns}/secrets/${name}" \
      --data-binary "$payload")" || return 1

    case "$http_code" in
    200 | 201) return 0 ;; # success
    *) return 1 ;;         # treat anything else as failure
    esac
  else
    kubectl patch secret -n "$ns" "$name" \
      --type merge \
      -p "$payload" \
      >/dev/null 2>&1
  fi
}

reconcile_secret() {
  ns="$1"
  client="$2"
  secret="$3"
  clientIdKey="$4"
  clientSecretKey="$5"

  secret_name="kanidm-${client}-oidc"
  payload="$(
    jq -nc --arg id "$client" --arg s "$secret" --arg ik "$clientIdKey" --arg sk "$clientSecretKey" \
      '{stringData: {$sk: $s, $ik: $id}}'
  )"

  # Try patching the secret first
  if ! k8s_patch_secret "$ns" "$secret_name" "$payload"; then
    # If patch failed (likely not found), create the secret
    if ! kubectl create secret generic -n "$ns" "$secret_name" \
      --from-literal="$clientIdKey=$client" \
      --from-literal="$clientSecretKey=$secret" \
      >/dev/null 2>&1; then
      log "warn: failed to create or patch secret $ns/$secret_name"
    else
      log "created secret $ns/$secret_name"
    fi
  else
    log "patched secret $ns/$secret_name"
  fi
}

wait_for_kanidm() {
  local url="${KANIDM_INSTANCE}/status"
  log "Waiting for Kanidm at $url"
  # Try until it responds with HTTP 200
  until curl -fsS "$url" >/dev/null 2>&1; do
    sleep 2
  done
  log "Kanidm is up"
}

get_basic_secret() {
  local rs
  rs="$1"
  curl -fsS \
    -H 'accept: application/json' \
    -H "Authorization: Bearer ${KANIDM_TOKEN}" \
    "${KANIDM_INSTANCE}/v1/oauth2/${rs}/_basic_secret" |
    jq -er '.'
}

reconcile() {
  # Run reconcile in a subshell; on any error, log and return success (non-fatal)
  (
    # everything inside still benefits from -euo pipefail
    local all ns_clients

    all="$(
      kubectl get configmaps -n "$NAMESPACE" -l "$LABEL_SELECTOR" -o json |
        jq -c --arg ns "$NAMESPACE" '
        import "lib" as l;
				.items
				| map({
					#name: .metadata.name,
					namespace: (.data.targetNamespace // $ns),
					data: (
						.data
						| to_entries
						| map(select(.key | endswith(".json")))
						| map(.value | fromjson)
						| reduce .[] as $x ({}; l::dmerge(.; $x))
					)
				})
			'
    )"

    ns_clients="$(
      printf '%s\n' "$all" |
        jq -c '
				sort_by(.namespace)
				| map(
				  .namespace as $ns
				  | .data.systems?.oauth2 // {}
				  | to_entries
				  | map({ name: .key, namespace: $ns, config: (.value.k8s? // {}) })
				) | add // []
			'
    )"

    mapfile -t lines < <(printf '%s\n' "$ns_clients" | jq -r '
        .[]? | [.name, .config?.imageUrl // ""] | @tsv
    ')

    images=()
    for line in "${lines[@]}"; do
      IFS=$'\t' read -r name imageUrl <<<"$line"
      icon="/data/icons/${name}.svg"

      if [[ -z "$imageUrl" ]]; then
        continue
      fi

      if [[ "$(curl -H "Authorization: Bearer ${KANIDM_TOKEN}" -s -o /dev/null -w "%{http_code}" "${KANIDM_INSTANCE}/ui/images/oauth2/${name}")" -eq 404 ]]; then
        if [[ -f "$icon" ]]; then
          images+=("${name}=${icon}")
          continue
        fi

        if ! curl -fL --retry 3 --retry-delay 5 -o "$icon" "$imageUrl" &>/dev/null; then
          log "Error: Failed to download $imageUrl"
          continue
        fi

        if [[ -f "$icon" ]]; then
          images+=("${name}=${icon}")
        fi
      fi
    done

    images_json="$(printf '%s\n' "${images[@]}" |
      jq -Rn '
        import "lib" as l;
        [inputs | split("=")] |
        if (.[0] | length) == 0 then {}
        else map({ systems:{oauth2:{(.[0]): {imageFile:.[1]}}} }) | reduce .[] as $x ({}; l::dmerge(.; $x))
        end
      ')"

    # Optional: provision from merged config over the base skeleton
    if [[ -x "/usr/local/bin/kanidm-provision" ]]; then
      log "Provisioning with merged state"

      if ! printf '%s\n' "$all" |
        jq -e -c --argjson base "$BASE" --argjson images "$images_json" '
    			$base * $images * (
    				map(.data) | reduce .[] as $item ({}; . * $item) | del(.systems.oauth2[]?.k8s)
    			)
    		' |
        KANIDM_TOKEN="$KANIDM_TOKEN" \
          kanidm-provision \
          --no-auto-remove \
          --url "$KANIDM_INSTANCE" \
          --state /dev/stdin; then
        log "warn: kanidm-provision failed"
      fi
    fi

    # Iterate namespace/client pairs; create/update secrets idempotently
    printf '%s\n' "$ns_clients" |
      jq -r '
			.[]?
			| [.namespace, .name, .config?.clientIdKey // "client-id", .config?.clientSecretKey // "client-secret"] | @tsv
		' |
      while IFS="$(printf '\t')" read -r ns client clientIdKey clientSecretKey; do
        if ! secret="$(get_basic_secret "$client")"; then
          log "warn: failed to fetch secret for client=$client (ns=$ns)"
          continue
        fi
        [ -n "$secret" ] || {
          log "warn: empty secret for client=$client (ns=$ns)"
          continue
        }

        reconcile_secret "$ns" "$client" "$secret" "$clientIdKey" "$clientSecretKey"
      done
  ) || {
    log "non-fatal: reconcile failed (will continue loop)"
    return 0
  }
}

use_incluster_sa

wait_for_kanidm

# Initial reconcile (non-fatal)
reconcile || log "non-fatal: initial reconcile failed"

# Watch loop: never exit the container on errors; restart the watch if it breaks
while true; do
  kubectl get configmaps -n "$NAMESPACE" -l "$LABEL_SELECTOR" --watch-only -o name |
    while read -r _; do
      while read -r -t "$DEBOUNCE_SECONDS" _; do :; done
      reconcile || log "non-fatal: reconcile failed during watch event"
    done
  log "watch stream ended or failed; restarting in 5s"
  sleep 5
done
