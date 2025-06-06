use anyhow::Context;
use clap::{Parser, Subcommand};
use dsd_util::printer::{color_println, color_println_fmt, Color};
use dsd_util::utils::{
    get_containers_from_stack, get_timestamp, kill_containers, list_containers,
    spawn_container_logger, update_container_by_name, use_color,
};
use dsd_util::DOCKER;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

pub const COMPOSE: &str = "compose";
pub const DSD: &str = "docker-stack-deploy";
pub const PATH_DSD_COMPOSE: &str = "/var/lib/docker-stack-deploy/compose.yml";
pub const DEFAULT_ARG_PROJECT_DIR: &str = "/var/lib/docker-stack-deploy";
pub const DEFAULT_ARG_TAIL: &str = "100";

#[derive(Debug, Parser)]
#[command(version, about = "A simple helper for managing your docker-stack-deploy containers.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialize and bootstrap a new instance of docker-stack-deploy
    Init {
        /// Path where docker-stack-deploy compose file will be located
        #[arg(long, default_value = DEFAULT_ARG_PROJECT_DIR)]
        project_dir: String,

        /// The git remote you want to utilize for docker-stack-deploy. Example: https://github.com/YOURNAME/REPO.git
        git_url: String,
    },

    // TODO: Add more arg options for logs - since, filter, follow ?
    /// View container logs
    Logs {
        /// View logs for specified containers
        containers: Option<Vec<String>>,

        /// View logs for specified stacks
        #[arg(long)]
        stacks: Option<Vec<String>>,

        /// Set the number of lines to show from end of logs
        #[arg(long, default_value = DEFAULT_ARG_TAIL)]
        tail: u32,

        /// View logs for all containers
        #[arg(long)]
        all: bool,
    },

    // TODO: confirm nuke
    /// Kill all docker containers and redeploy docker-stack-deploy
    Nuke,

    /// Restart containers
    Restart {
        /// Restart specified container
        containers: Option<Vec<String>>,

        /// Restart specified stacks
        #[arg(long)]
        stacks: Option<Vec<String>>,

        /// Restart all containers
        #[arg(long)]
        all: bool,
    },

    /// View basic stats for docker containers
    Stats {
        /// View stats for specified containers
        containers: Option<Vec<String>>,

        /// View stats for specified stacks
        #[arg(long)]
        stacks: Option<Vec<String>>,

        /// View stats for all containers
        #[arg(long)]
        all: bool,
    },

    /// Update container images
    Update {
        /// Update specified containers
        containers: Option<Vec<String>>,

        /// Update specified stacks
        #[arg(long)]
        stacks: Option<Vec<String>>,

        /// Update all containers
        #[arg(long)]
        all: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            project_dir,
            git_url,
        } => init(project_dir, git_url)?,
        Commands::Logs {
            containers,
            stacks,
            tail,
            all,
        } => logs(containers, stacks, tail, all)?,
        Commands::Nuke => nuke()?,
        Commands::Restart {
            containers,
            stacks,
            all,
        } => restart(containers, stacks, all)?,
        Commands::Stats {
            containers,
            stacks,
            all,
        } => stats(containers, stacks, all)?,
        Commands::Update {
            containers,
            stacks,
            all,
        } => update(containers, stacks, all)?,
    }

    Ok(())
}

/// Initializes a new instance of docker-stack-deploy using bootstrap script
fn init(project_dir: String, git_url: String) -> anyhow::Result<()> {
    Command::new(DOCKER)
        .args(["run", "--rm", "-it"])
        .args(["-v", "/var/run/docker.sock:/var/run/docker.sock"])
        .args(["-v", &format!("{project_dir}:{project_dir}")])
        .args(["ghcr.io/wez/docker-stack-deploy"])
        .args([DSD, "bootstrap"])
        .args(["--project-dir", &project_dir])
        .args(["--git-url", &git_url])
        .status()
        .context("Failed to bootstrap docker-stack-deploy")?;

    println!();

    let use_color = use_color();

    if use_color {
        color_println(
            Color::Green,
            "Bootstrap success! Following docker-stack-deploy logs...",
        );
    } else {
        println!("Bootstrap success! Following docker-stack-deploy logs...")
    }

    println!();

    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("Failed to get current time")?
        .as_secs();

    // follow docker-stack-deploy logs until first update check has happened
    let mut logs_process = Command::new(DOCKER)
        .args([
            "compose",
            "-f",
            PATH_DSD_COMPOSE,
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

/// Shows logs for specified containers
fn logs(
    containers: Option<Vec<String>>,
    stacks: Option<Vec<String>>,
    tail: u32,
    all: bool,
) -> anyhow::Result<()> {
    let use_color = use_color();

    let containers = if all {
        let container_ids = list_containers()?;

        if container_ids.is_empty() {
            if use_color {
                color_println(Color::Red, "No containers running");
            } else {
                println!("No containers running");
            }
            return Ok(());
        }

        container_ids
    } else if let Some(containers) = containers {
        containers
    } else if let Some(stacks) = stacks {
        let mut containers = vec![];

        for stack in &stacks {
            let container_names = get_containers_from_stack(stack)?;
            containers.extend(container_names);
        }

        containers
    } else {
        anyhow::bail!("Must specify containers or use --all (-a)")
    };

    if use_color {
        color_println(
            Color::Cyan,
            &format!("Following logs for container: {}", &containers.len()),
        );
    } else {
        println!("Following logs for container: {}", &containers.len());
    }
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut handles: Vec<std::thread::JoinHandle<()>> = vec![];

    for container in containers {
        let tx = tx.clone();
        let is_container_id = all;
        let handle = spawn_container_logger(&container, is_container_id, use_color, tail, tx)
            .with_context(|| format!("Failed to spawn container logger for {}", container))?;
        handles.push(handle);
    }

    drop(tx);

    for log_line in rx {
        println!("{log_line}");
    }

    for handle in handles {
        let _ = handle.join();
    }

    Ok(())
}

/// Kills all running containers, and then redeploys docker-stack-deploy
fn nuke() -> anyhow::Result<()> {
    // get list of currently running docker containers by id
    let container_ids = list_containers()?;

    // if docker containers are running, kill them
    if !container_ids.is_empty() {
        kill_containers(container_ids)?
    }

    let use_color = use_color();

    if use_color {
        color_println(Color::Green, "Running docker-stack-deploy...");
    } else {
        println!("Running docker-stack-deploy...")
    }

    // run docker-stack-deploy
    Command::new(DOCKER)
        .args(["compose", "-f", PATH_DSD_COMPOSE, "up", "-d"])
        .status()
        .context("Failed to start docker-stack-deploy")?;

    if use_color {
        color_println(
            Color::Green,
            "Following logs until all containers deployed...",
        );
    } else {
        println!("Following logs until all containers deployed...")
    }

    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("Failed to get current time")?
        .as_secs();

    // follow docker-stack-deploy logs until first update check has happened
    let mut logs_process = Command::new(DOCKER)
        .args([
            "compose",
            "-f",
            PATH_DSD_COMPOSE,
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

/// Restarts specified docker containers
fn restart(
    containers: Option<Vec<String>>,
    stacks: Option<Vec<String>>,
    all: bool,
) -> anyhow::Result<()> {
    let containers = if all {
        list_containers()?
    } else if let Some(containers) = containers {
        containers
    } else if let Some(stacks) = stacks {
        let mut containers = vec![];

        for stack in &stacks {
            let container_names = get_containers_from_stack(stack)?;
            containers.extend(container_names);
        }

        containers
    } else {
        anyhow::bail!("Must specify containers or use --all (-a)")
    };

    let use_color = use_color();

    for container in &containers {
        if use_color {
            color_println(
                Color::Cyan,
                &format!("Restarting container: {}", &container),
            );
        } else {
            println!("Restarting container: {}", &container)
        }

        Command::new(DOCKER)
            .args(["restart", container])
            .status()
            .context(format!("Failed to restart {}", &container))?;
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct ContainerStats {
    name: String,
    image: String,
    status: String,
    restart_policy: String,
    cpu_usage: String,
    memory_usage: String,
    ip_address: String,
}
/// View stats for docker containers
fn stats(
    containers: Option<Vec<String>>,
    stacks: Option<Vec<String>>,
    all: bool,
) -> anyhow::Result<()> {
    let use_color = use_color();
    let containers = if all {
        let container_ids = list_containers()?;

        if container_ids.is_empty() {
            if use_color {
                color_println(Color::Red, "No containers running");
            } else {
                println!("No containers running");
            }
            return Ok(());
        }

        container_ids
    } else if let Some(containers) = containers {
        containers
    } else if let Some(stacks) = stacks {
        let mut containers = vec![];

        for stack in &stacks {
            let container_names = get_containers_from_stack(stack)?;
            containers.extend(container_names);
        }

        containers
    } else {
        anyhow::bail!("Must specify containers, use --stacks (-s) or use --all (-a)")
    };

    let stats_output = Command::new(DOCKER)
        .args([
            "stats",
            "--no-stream",
            "--format",
            "table {{.Name}}\t{{.CPUPerc}}\t{{.MemPerc}}",
        ])
        .args(&containers)
        .output()
        .context("Failed to get stats for containers")?;

    let inspect_output = Command::new(DOCKER)
        .arg("inspect")
        .args(&containers)
        .args(["--format", "{{.Name}},{{.Config.Image}},{{.State.Status}},{{if .HostConfig.RestartPolicy}}{{if .HostConfig.RestartPolicy.Name}}{{.HostConfig.RestartPolicy.Name}}{{else}}no{{end}}{{else}}no{{end}},{{if .NetworkSettings.IPAddress}}{{.NetworkSettings.IPAddress}}{{else}}N/A{{end}}"])
        .output().context("Failed to inspect containers")?;

    let stats_string = String::from_utf8(stats_output.stdout)?;
    let inspect_string = String::from_utf8(inspect_output.stdout)?;

    let mut temp_stats_map: std::collections::HashMap<String, StatsData> =
        std::collections::HashMap::new();
    let mut temp_inspect_map: std::collections::HashMap<String, InspectData> =
        std::collections::HashMap::new();

    // skip header line
    for line in stats_string.lines().skip(1) {
        let parsed = parse_stats_data(line)?;
        temp_stats_map.insert(
            parsed.container_name.clone(),
            StatsData {
                container_name: parsed.container_name,
                cpu: parsed.cpu,
                memory: parsed.memory,
            },
        );
    }

    for line in inspect_string.lines() {
        let parsed = parse_inspect_data(line)?;
        temp_inspect_map.insert(
            parsed.container_name.clone(),
            InspectData {
                container_name: parsed.container_name,
                image: parsed.image,
                status: parsed.status,
                restart_policy: parsed.restart_policy,
                ip_address: parsed.ip_address,
            },
        );
    }

    assert_eq!(&temp_stats_map.len(), &temp_inspect_map.len());

    let mut total_stats_map: std::collections::HashMap<String, ContainerStats> =
        std::collections::HashMap::new();

    for key in temp_stats_map.keys() {
        let stats = temp_stats_map
            .get(key)
            .with_context(|| format!("Failed to get stats for {}", key))?;
        let inspect = temp_inspect_map
            .get(key)
            .with_context(|| format!("Failed to get stats for {}", key))?;

        let container_stats = if use_color {
            ContainerStats {
                name: color_println_fmt(Color::Green, &stats.container_name),
                image: inspect.image.to_string(),
                status: inspect.status.to_string(),
                restart_policy: inspect.restart_policy.to_string(),
                cpu_usage: stats.cpu.to_string(),
                memory_usage: stats.memory.to_string(),
                ip_address: inspect.ip_address.to_string(),
            }
        } else {
            ContainerStats {
                name: stats.container_name.to_string(),
                image: inspect.image.to_string(),
                status: inspect.status.to_string(),
                restart_policy: inspect.restart_policy.to_string(),
                cpu_usage: stats.cpu.to_string(),
                memory_usage: stats.memory.to_string(),
                ip_address: inspect.ip_address.to_string(),
            }
        };

        total_stats_map.insert(key.to_string(), container_stats);
    }

    println!(
        "{:<30} {:<50} {:<15} {:<20} {:<10} {:<10} {:<15}",
        "NAME", "IMAGE", "STATUS", "RESTART", "CPU %", "MEM %", "IP"
    );

    println!();

    for key in total_stats_map.keys() {
        let container = total_stats_map.get(key).context("Failed to get item")?;

        println!(
            "{:<30} {:<50} {:<15} {:<20} {:<10} {:<10} {:<15}",
            container.name,
            container
                .image
                .split("@")
                .next()
                .unwrap_or(&container.image),
            container.status,
            container.restart_policy,
            container.cpu_usage,
            container.memory_usage,
            container.ip_address
        );
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct StatsData {
    container_name: String,
    cpu: String,
    memory: String,
}
fn parse_stats_data(stats: &str) -> anyhow::Result<StatsData> {
    let parsed = stats
        .trim_start_matches("/")
        .split_whitespace()
        .collect::<Vec<&str>>();

    Ok(StatsData {
        container_name: parsed[0].to_string(),
        cpu: parsed[1].to_string(),
        memory: parsed[2].to_string(),
    })
}

#[derive(Debug, Clone)]
struct InspectData {
    container_name: String,
    image: String,
    status: String,
    restart_policy: String,
    ip_address: String,
}
fn parse_inspect_data(stats: &str) -> anyhow::Result<InspectData> {
    let parsed = stats
        .trim_start_matches("/")
        .split(",")
        .collect::<Vec<&str>>();

    Ok(InspectData {
        container_name: parsed[0].to_string(),
        image: parsed[1].to_string(),
        status: parsed[2].to_string(),
        restart_policy: parsed[3].to_string(),
        ip_address: parsed[4].to_string(),
    })
}

/// Updates images of specified docker containers
fn update(
    containers: Option<Vec<String>>,
    stacks: Option<Vec<String>>,
    all: bool,
) -> anyhow::Result<()> {
    let containers = if all {
        list_containers()?
    } else if let Some(containers) = containers {
        containers
    } else if let Some(stacks) = stacks {
        let mut containers = vec![];

        for stack in &stacks {
            let container_names = get_containers_from_stack(stack)?;
            containers.extend(container_names);
        }

        containers
    } else {
        anyhow::bail!("Must specify containers or use --all (-a)")
    };

    let use_color = use_color();

    let mut num_containers_updated = 0;

    for container in &containers {
        num_containers_updated += update_container_by_name(container)?;
    }

    if num_containers_updated == 0 {
        if use_color {
            color_println(Color::Yellow, "No new container images to update");
        } else {
            println!("No new container images to pull");
        }

        return Ok(());
    }

    if use_color {
        println!(
            "{}: {}",
            &color_println_fmt(Color::Cyan, "New images pulled"),
            &color_println_fmt(Color::Green, &num_containers_updated.to_string())
        );
        println!();
        color_println(Color::Green, &format!("Restarting {DSD}"));
    } else {
        println!("New images pulled: {}", num_containers_updated);
        println!();
        println!("Restarting {DSD}");
    }

    // containers updated, restart docker-stack-deploy to deploy new image
    Command::new(DOCKER)
        .args(["restart", DSD])
        .status()
        .context(format!("Failed to restart {DSD}"))?;

    Ok(())
}
