# The Conn: Tube

`tube` is the primary execution agent for **The Conn**, an OpenShift-native CI/CD platform. It is a statically-linked Rust binary designed to run as the entrypoint for containerized jobs, managing the lifecycle of a single pipeline node.

## Overview

The binary acts as a "ghost" wrapper around user-defined workloads. By using `musl` for static compilation, `tube` can be injected into any container image (from Alpine to UBI) without requiring pre-installed dependencies or shared libraries. It handles the secure retrieval of source code, manages the execution of scripts, and reports results back to the orchestration layer.

## Lifecycle

A single execution run follows a deterministic sequence:

1.  **Bootstrap**: Load configuration from environment variables, initialize the `tracing` subscriber, and load any mounted secrets from disk.
2.  **Status (Started)**: Report the initiation of the node to the S3-compatible status store using a pre-signed `PUT` URL.
3.  **Workspace Provisioning**: Stream the repository archive from a pre-signed URL. The binary auto-detects `gzip` or `zstd` from the URL, performs on-the-fly decompression, and extracts the archive into the workspace directory (stripping the leading repo-and-commit directory the backend wraps around the contents).
4.  **Execution**: Spawn the user-provided script (typically mounted via a ConfigMap) as a child process under `sh -c`, with secrets injected as environment variables. `stdout` and `stderr` are captured line by line, masked against secret values, emitted to `tracing`, and appended to an in-memory log buffer (capped at 10 MiB with FIFO eviction). A background task periodically `PUT`s the buffer snapshot to the logs URL.
5.  **Status (Finished)**: Stop the periodic uploader, push the final log buffer to the logs URL, and `PUT` a JSON payload to the status URL containing the success/failure state and timing metadata.
6.  **Notification (Poke)**: Issue a `POST` request to the backend API to trigger the next stage of the pipeline.

## Configuration

The binary utilizes a hierarchical configuration model. In production, values are injected via environment variables following the `TUBE__` prefix convention.

### Configuration Schema

```toml
[execution]
user_script_path = "/tmp/user_script.sh"
status_put_url = ""           # Pre-signed S3 URL for status updates (PUT)
logs_put_url = ""             # Pre-signed S3 URL for log uploads (PUT)
poke_url = ""                 # Backend API notification endpoint (POST)
run_id = ""                   # Unique identifier for the pipeline run
node_name = ""                # Name of the specific execution node
log_level = "info"            # User script log verbosity: trace, debug, info, warn, error
log_upload_interval_ms = 5000 # Interval between periodic log buffer uploads

[log]
level = "warn"                # Tube's own log verbosity: trace, debug, info, warn, error

[workspace]
get_url = ""                  # Pre-signed S3 URL for source retrieval (GET)
dir = "/workspace"

[secrets]
dir = "/etc/the-conn/secrets" # Directory of secret files (one per secret)
```

The `[execution].log_level` controls the verbosity of captured user-script output, while `[log].level` controls Tube's own internal logs. Both are filtered through a single `tracing` subscriber.

### Environment Variable Mapping

The configuration paths map directly to environment variables using double underscores as separators. For example:

* `TUBE__EXECUTION__STATUS_PUT_URL`
* `TUBE__EXECUTION__LOGS_PUT_URL`
* `TUBE__EXECUTION__POKE_URL`
* `TUBE__EXECUTION__RUN_ID`
* `TUBE__EXECUTION__NODE_NAME`
* `TUBE__EXECUTION__USER_SCRIPT_PATH`
* `TUBE__WORKSPACE__GET_URL`
* `TUBE__WORKSPACE__DIR`
* `TUBE__SECRETS__DIR`

## Secrets

In the production runtime, secrets are populated by a Vault sidecar/init container that materializes them onto a shared volume before `tube` starts. `tube` itself only reads files from disk — it has no Vault client built in — which keeps the binary small and lets the secret backend be swapped without changing the agent.

### Env-var secrets

`tube` walks `TUBE__SECRETS__DIR` (default `/etc/tube/secrets/env`) and treats each file as one environment variable: the filename is the env-var name and the file body (after trimming surrounding whitespace) is the value. For example, `/etc/tube/secrets/env/QUAY_USERNAME` and `/etc/tube/secrets/env/QUAY_PASSWORD` become `$QUAY_USERNAME` and `$QUAY_PASSWORD` in the user script's environment.

Behavior:

* The directory is optional. If it does not exist, `tube` runs without secrets.
* Files whose names start with `.` (e.g. the `..data` symlink in Kubernetes secret mounts) are ignored.
* Empty values (after trimming) are skipped.
* Every captured stdout/stderr line is scanned and any occurrence of a secret value is replaced with `***` before the line is appended to the log buffer or emitted to tracing. The S3 log upload therefore never contains plaintext secrets, even if the user script accidentally echoes them.

### File secrets

Vault can also project secrets directly as files for the user script to consume (e.g. TLS material, SSH keys, kubeconfigs). The default mount root is `/etc/tube/secrets/files/` (e.g. `cat /etc/tube/secrets/files/KUBECONFIG`), and individual file secrets can also be projected to arbitrary paths chosen at job-definition time. `tube` does not enumerate or interpret these files — it neither injects them as env vars nor knows their contents.

**File-secret values are not masked in logs.** The masking pass only runs against env-var secret values that `tube` loaded from `TUBE__SECRETS__DIR`. If a user script `cat`s a file secret to stdout, or otherwise echoes its contents, the value will appear verbatim in the captured logs and the S3 upload. Treat file secrets as "files the user script is trusted to handle" rather than as opaquely masked material.

## Security Model

`tube` enforces data isolation by design. The execution pod is never granted broad access to the S3 bucket or GitHub credentials. All external data access is mediated through short-lived, pre-signed URLs generated by the backend. This ensures that even if a user script is compromised, it cannot access repositories or status data belonging to other pipeline runs.

## Development

### Building
For a local host-architecture build (dynamic, useful for development):

```bash
make build
```

For the production artifact, use the multi-stage `Containerfile`, which compiles for the static `x86_64-unknown-linux-musl` target and produces an Alpine-based image suitable for use as a Carrier init container:

```bash
make image
```

### Deployment
To use the binary in an execution job, use a "Carrier" init container to copy the binary into a shared `emptyDir` volume. The image's default `CMD` already runs `cp /usr/local/bin/tube /shared/tube`, so mounting the shared volume at `/shared` is enough:

```yaml
initContainers:
  - name: install-tube
    image: quay.io/the-conn/tube:latest
    volumeMounts:
      - name: bin-volume
        mountPath: /shared
```
