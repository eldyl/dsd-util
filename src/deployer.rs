// Reference to compose file in the docker-stack-deploy repo:
// https://github.com/wez/docker-stack-deploy/blob/main/compose.yml

use crate::commands::DOCKER;
use crate::printer::{color_println_fmt, Color};
use crate::utils::get_timestamp;
use anyhow::Context;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

const DSD: &str = "docker-stack-deploy";

/// Resolves the docker host socket path from DOCKER_HOST or falls back to the default
pub fn resolve_host_sock() -> anyhow::Result<String> {
    match std::env::var("DOCKER_HOST") {
        Ok(s) => {
            let Some(path) = s.strip_prefix("unix://") else {
                anyhow::bail!("dsd-util only supports unix:// DOCKER_HOST. Got: {s}");
            };
            Ok(path.to_string())
        }
        Err(_) => Ok("/var/run/docker.sock".to_string()),
    }
}

/// Default project_dir: user-level XDG path when docker appears user-scoped
/// (any unix:// DOCKER_HOST other than the default rootful sock), /var/lib otherwise.
///
/// This covers rootless setups regardless of whether the socket lives under
/// /run/user/ (systemd), $XDG_RUNTIME_DIR elsewhere, or a user-chosen path.
pub fn default_project_dir() -> String {
    compute_default_project_dir(
        std::env::var("DOCKER_HOST").ok().as_deref(),
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").unwrap_or_default().as_str(),
    )
}

/// Pure resolver for default_project_dir — takes explicit env slices so it's testable
/// without mutating process-global env.
fn compute_default_project_dir(
    docker_host: Option<&str>,
    xdg_data_home: Option<&str>,
    home: &str,
) -> String {
    let is_user_scoped = match docker_host {
        Some(v) => match v.strip_prefix("unix://") {
            Some(path) => !path.is_empty() && path != "/var/run/docker.sock",
            None => false,
        },
        None => false,
    };

    if is_user_scoped {
        let base = xdg_data_home
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("{home}/.local/share"));
        format!("{base}/docker-stack-deploy")
    } else {
        "/var/lib/docker-stack-deploy".to_string()
    }
}

/// Path to the generated deployer compose.yml within a given project_dir
pub fn compose_path(project_dir: &str) -> String {
    format!("{project_dir}/compose.yml")
}

/// Path to the deployer .env file within a given project_dir (colocated with compose.yml)
pub fn env_path(project_dir: &str) -> String {
    format!("{project_dir}/.env")
}

/// Shape of the deployer compose.yml
#[derive(Serialize)]
struct ComposeFile {
    name: &'static str,
    services: BTreeMap<&'static str, DeployerService>,
}

/// Shape of the `deployer` service within the compose file
#[derive(Serialize)]
struct DeployerService {
    image: &'static str,
    container_name: &'static str,
    restart: &'static str,
    uts: &'static str,
    env_file: &'static str,
    environment: Vec<String>,
    volumes: Vec<String>,
}

/// Serializes the deployer compose.yml as a YAML string
pub fn render_compose_yaml(project_dir: &str, host_sock: &str) -> anyhow::Result<String> {
    let environment = vec![
        format!("STACK_REPO_DIR={project_dir}/repo"),
        format!("DOCKER_SOCK_HOST={host_sock}"),
    ];

    let volumes = vec![
        format!("{host_sock}:/var/run/docker.sock"),
        format!("{project_dir}:{project_dir}"),
    ];

    let mut services = BTreeMap::new();
    services.insert(
        "deployer",
        DeployerService {
            image: "ghcr.io/wez/docker-stack-deploy",
            container_name: "docker-stack-deploy",
            restart: "always",
            uts: "host",
            env_file: ".env",
            environment,
            volumes,
        },
    );

    let compose = ComposeFile {
        name: "docker-stack-deploy",
        services,
    };

    serde_yml::to_string(&compose).context("Failed to serialize compose.yml")
}

/// Writes the deployer compose.yml to disk, overwriting any existing file
pub fn write_compose_yaml(project_dir: &str, host_sock: &str) -> anyhow::Result<()> {
    let path = compose_path(project_dir);
    let yaml = render_compose_yaml(project_dir, host_sock)?;

    fs::write(&path, yaml).context(format!("Failed to write {}", &path))?;

    Ok(())
}

/// Writes the deployer .env file if missing
pub fn ensure_env_file(project_dir: &str, git_url: &str) -> anyhow::Result<()> {
    let path = env_path(project_dir);

    // preserve existing .env - operator can delete it to rotate creds or change repo URL
    if Path::new(&path).exists() {
        if let Ok(contents) = fs::read_to_string(&path) {
            let existing_url = contents
                .lines()
                .find_map(|l| l.strip_prefix("GITHUB_URL="))
                .map(|v| v.trim().trim_matches('"'));
            if let Some(existing) = existing_url
                && existing != git_url
            {
                eprintln!(
                    "warning: {path} already has GITHUB_URL={existing:?}; CLI-provided {git_url:?} ignored. Delete {path} to change repo URL."
                );
            }
        }
        return Ok(());
    }

    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "{path} is missing and stdin is not a TTY - run `dsd-util init` interactively to create it"
        );
    }

    let github_url = git_url;
    let github_username =
        std::env::var("GITHUB_USERNAME").unwrap_or_else(|_| "oauth2".to_string());
    let poll_interval = std::env::var("POLL_INTERVAL").unwrap_or_else(|_| "300".to_string());

    let github_token = prompt_secret("GITHUB_TOKEN")?;
    let stack_kdbx_pass = prompt_secret("STACK_KDBX_PASS")?;

    // quote values to match upstream bootstrap's .env format
    let contents = format!(
        "GITHUB_URL=\"{github_url}\"\n\
         GITHUB_USERNAME=\"{github_username}\"\n\
         GITHUB_TOKEN=\"{github_token}\"\n\
         STACK_KDBX_PASS=\"{stack_kdbx_pass}\"\n\
         POLL_INTERVAL=\"{poll_interval}\"\n"
    );

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .context(format!("Failed to create {}", &path))?;
    file.write_all(contents.as_bytes())
        .context(format!("Failed to write {}", &path))?;

    Ok(())
}

/// Prompts on stdin for a secret value with input masking (no echo)
fn prompt_secret(key: &str) -> anyhow::Result<String> {
    let value =
        rpassword::prompt_password(format!("{key}: ")).context(format!("Failed to read {key}"))?;
    Ok(value)
}

/// Brings up the deployer container via `docker compose up -d`
pub fn bring_up(project_dir: &str) -> anyhow::Result<()> {
    let status = Command::new(DOCKER)
        .args(["compose", "-f", &compose_path(project_dir), "up", "-d"])
        .status()
        .context("Failed to start docker-stack-deploy")?;

    if !status.success() {
        anyhow::bail!("docker compose up -d failed with status {status}");
    }

    Ok(())
}

/// Follows deployer logs until the first "Already up to date" line after deploy
pub fn follow_deploy_logs(project_dir: &str, use_color: bool) -> anyhow::Result<()> {
    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("Failed to get current time")?
        .as_secs();

    // follow docker-stack-deploy logs until first update check has happened
    let mut logs_process = Command::new(DOCKER)
        .args([
            "compose",
            "-f",
            &compose_path(project_dir),
            "logs",
            "--follow",
            "--no-log-prefix",
            "--since",
            &start_time.to_string(),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to start following logs")?;

    if let Some(stdout) = logs_process.stdout.take() {
        let reader = BufReader::new(stdout);
        for (i, line) in reader.lines().map_while(Result::ok).enumerate() {
            if use_color {
                println!(
                    "[{} | {}] {}",
                    color_println_fmt(Color::Cyan, &get_timestamp()),
                    color_println_fmt(Color::Magenta, DSD),
                    line
                );
            } else {
                println!("[{} | {}] {}", &get_timestamp(), DSD, line);
            }
            if line.contains("Already up to date") && i > 0 {
                // first update check has happened after deployment
                break;
            }
        }
    }

    let _ = logs_process.kill();
    let _ = logs_process.wait();

    Ok(())
}
