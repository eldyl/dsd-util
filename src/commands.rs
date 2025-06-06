use crate::printer::{color_println, color_println_fmt, Color};
use crate::utils::{
    get_containers_from_stack, get_timestamp, is_terminal, kill_containers, list_containers,
    parse_inspect_data, parse_stats_data, spawn_container_logger, update_container_by_name,
    InspectData, StatsData,
};
use anyhow::Context;
use std::collections::hash_map::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};

pub const DOCKER: &str = "docker";
const DSD: &str = "docker-stack-deploy";
const PATH_DSD_COMPOSE: &str = "/var/lib/docker-stack-deploy/compose.yml";

/// Initializes a new instance of docker-stack-deploy using bootstrap script
pub fn init(project_dir: String, git_url: String) -> anyhow::Result<()> {
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

    let use_color = is_terminal();

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
pub fn logs(
    containers: Option<Vec<String>>,
    stacks: Option<Vec<String>>,
    tail: u32,
    all: bool,
) -> anyhow::Result<()> {
    let use_color = is_terminal();

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
            .with_context(|| format!("Failed to spawn container logger for {container}"))?;
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
pub fn nuke() -> anyhow::Result<()> {
    // ask user to confirm action
    color_println(
        Color::Yellow,
        "WARNING: All of your containers will be forcefully removed!",
    );
    println!(
        "After removal, {} will be restarted to redeploy all associated containers.\n",
        color_println_fmt(Color::Magenta, DSD)
    );
    print!("Are you sure you want to nuke your docker stacks? [y/N]: ");
    let _ = io::stdout().flush();

    // capture user input
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
    let response = input.trim().to_lowercase();

    // evaluate response
    match response.as_str() {
        "yes" | "y" => {
            color_println(Color::Yellow, "Nuking docker containers");
        }
        _ => {
            color_println(Color::Green, "Nuke aborted!");
            return Ok(());
        }
    };

    // get list of currently running docker containers by id
    let container_ids = list_containers()?;

    // if docker containers are running, kill them
    if container_ids.is_empty() {
        color_println(Color::Red, "No containers running");
        return Ok(());
    } else {
        kill_containers(container_ids)?
    }

    color_println(Color::Green, "Running docker-stack-deploy...");

    // run docker-stack-deploy
    Command::new(DOCKER)
        .args(["compose", "-f", PATH_DSD_COMPOSE, "up", "-d"])
        .status()
        .context("Failed to start docker-stack-deploy")?;

    color_println(
        Color::Green,
        "Following logs until all containers deployed...",
    );

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
            println!(
                "[{} | {}] {}",
                color_println_fmt(Color::Cyan, &get_timestamp()),
                color_println_fmt(Color::Magenta, DSD),
                line
            );
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
pub fn restart(
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
        anyhow::bail!("Must specify containers, use --stacks (-s) or use --all (-a)")
    };

    let use_color = is_terminal();

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

/// Container stats to be gathered
#[derive(Debug, Clone)]
struct ContainerStats {
    name: String,
    status: String,
    health: String,
    restart_policy: String,
    uptime: String,
    cpu_usage: String,
    memory_usage: String,
    ports: String,
}

/// View stats for docker containers
pub fn stats(
    containers: Option<Vec<String>>,
    stacks: Option<Vec<String>>,
    all: bool,
) -> anyhow::Result<()> {
    let use_color = is_terminal();
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

    let inspect_format = concat!(
        "{{.Name}},",
        "{{.State.Status}},",
        "{{if .HostConfig.RestartPolicy}}{{if .HostConfig.RestartPolicy.Name}}{{.HostConfig.RestartPolicy.Name}}{{else}}no{{end}}{{else}}no{{end}},",
        "{{if index .State \"Health\"}}{{.State.Health.Status}}{{else}}N/A{{end}},",
        "{{.State.StartedAt}},",
        "{{if .NetworkSettings.Ports}}{{range $key, $value := .NetworkSettings.Ports}}{{$key}}{{if $value}}:{{(index $value 0).HostPort}}{{end}} {{end}}{{else}}N/A{{end}}"
        );

    let inspect_output = Command::new(DOCKER)
        .arg("inspect")
        .args(&containers)
        .args(["--format", inspect_format])
        .output()
        .context("Failed to inspect containers")?;

    let stats_string = String::from_utf8(stats_output.stdout)?;
    let inspect_string = String::from_utf8(inspect_output.stdout)?;

    let mut temp_stats_map: HashMap<String, StatsData> = HashMap::new();
    let mut temp_inspect_map: HashMap<String, InspectData> = HashMap::new();

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
                status: parsed.status,
                restart_policy: parsed.restart_policy,
                health: parsed.health,
                uptime: parsed.uptime,
                ports: parsed.ports,
            },
        );
    }

    assert_eq!(&temp_stats_map.len(), &temp_inspect_map.len());

    let mut total_stats_map: HashMap<String, ContainerStats> = HashMap::new();

    for key in temp_stats_map.keys() {
        let stats = temp_stats_map
            .get(key)
            .with_context(|| format!("Failed to get stats for {key}"))?;
        let inspect = temp_inspect_map
            .get(key)
            .with_context(|| format!("Failed to get stats for {key}"))?;

        let container_stats = if use_color {
            ContainerStats {
                name: color_println_fmt(Color::Cyan, &stats.container_name),
                status: {
                    if &inspect.status.to_lowercase() == "running" {
                        color_println_fmt(Color::Green, &inspect.status)
                    } else if &inspect.status.to_lowercase() == "created" {
                        color_println_fmt(Color::Cyan, &inspect.status)
                    } else if &inspect.status.to_lowercase() == "paused"
                        || &inspect.status.to_lowercase() == "restarting"
                    {
                        color_println_fmt(Color::Yellow, &inspect.status)
                    } else {
                        color_println_fmt(Color::Red, &inspect.status)
                    }
                },
                restart_policy: inspect.restart_policy.to_string(),
                health: {
                    if &inspect.health.to_lowercase() == "healthy" {
                        color_println_fmt(Color::Green, &inspect.health)
                    } else if &inspect.health.to_lowercase() == "unhealthy" {
                        color_println_fmt(Color::Red, &inspect.health)
                    } else if &inspect.health.to_lowercase() == "starting" {
                        color_println_fmt(Color::Cyan, &inspect.health)
                    } else {
                        color_println_fmt(Color::White, &inspect.health)
                    }
                },
                uptime: inspect.uptime.to_string(),
                cpu_usage: stats.cpu.to_string(),
                memory_usage: stats.memory.to_string(),
                ports: inspect.ports.to_string(),
            }
        } else {
            ContainerStats {
                name: stats.container_name.to_string(),
                status: inspect.status.to_string(),
                restart_policy: inspect.restart_policy.to_string(),
                health: inspect.health.to_string(),
                uptime: inspect.uptime.to_string(),
                cpu_usage: stats.cpu.to_string(),
                memory_usage: stats.memory.to_string(),
                ports: inspect.ports.to_string(),
            }
        };

        total_stats_map.insert(key.to_string(), container_stats);
    }
    if use_color {
        println!(
            "{:<35} {:<20} {:<16} {:<20} {:<18} {:<8} {:<8} {:<20}",
            &color_println_fmt(Color::White, "NAME"),
            &color_println_fmt(Color::White, "STATUS"),
            "RESTART",
            &color_println_fmt(Color::White, "HEALTH"),
            "UPTIME",
            "CPU %",
            "MEM %",
            "PORTS"
        );
    } else {
        println!(
            "{:<35} {:<20} {:<16} {:<20} {:<18} {:<8} {:<8} {:<20}",
            "NAME", "STATUS", "RESTART", "HEALTH", "UPTIME", "CPU %", "MEM %", "PORTS"
        );
    }

    println!();

    // TODO: sort - probably want to use BTreeMap instead
    for key in total_stats_map.keys() {
        let container = total_stats_map.get(key).context("Failed to get item")?;

        println!(
            "{:<35} {:<20} {:<16} {:<20} {:<18} {:<8} {:<8} {:<20}",
            container.name,
            container.status,
            container.restart_policy,
            container.health,
            container.uptime,
            container.cpu_usage,
            container.memory_usage,
            container.ports
        );
    }

    Ok(())
}

/// Updates images of specified docker containers
pub fn update(
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
        anyhow::bail!("Must specify containers, use --stacks (-s) or use --all (-a)")
    };

    let use_color = is_terminal();

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
        println!("New images pulled: {num_containers_updated}");
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
