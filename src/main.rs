use anyhow::Context;
use chrono::Local;
use clap::{Parser, Subcommand};
use dsp_util::{color_println, color_println_fmt, Color};
use std::io::{BufRead, BufReader, IsTerminal};
use std::process::{Command, Stdio};

const DOCKER: &str = "docker";
const DSP: &str = "docker-stack-deploy";
const PATH_DSP_COMPOSE: &str = "/var/lib/docker-stack-deploy/compose.yml";

#[derive(Debug, Parser)]
#[command(version, about = "A simple helper for managing your docker-stack-deploy containers.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

// TODO: Add init feature - allow utility to setup an initial instance of docker-stack-deploy
// $ docker run --rm -it \
//     -v /var/run/docker.sock:/var/run/docker.sock \
//     -v /var/lib/docker-stack-deploy:/var/lib/docker-stack-deploy \
//     ghcr.io/wez/docker-stack-deploy \
//     docker-stack-deploy bootstrap \
//     --project-dir /var/lib/docker-stack-deploy \
//     --git-url https://github.com/YOURNAME/REPO.git

#[derive(Debug, Subcommand)]
enum Commands {
    // TODO: Add more arg options for logs - since, ?
    /// View container logs
    Logs {
        /// View logs for specified container
        containers: Option<Vec<String>>,

        /// Set the number of lines to show from end of logs
        #[arg(long, default_value = "100")]
        tail: u32,

        /// View logs for all containers
        #[arg(long)]
        all: bool,
    },

    /// Kill all docker containers and redeploy docker-stack-deploy
    Nuke,

    /// Restart containers
    Restart {
        /// Restart specified container
        containers: Option<Vec<String>>,

        /// Restart all containers
        #[arg(long)]
        all: bool,
    },

    // OPTIMIZE: Don't restart docker-stack-deploy if no containers were updated
    /// Update containers
    Update {
        /// Update specified container
        containers: Option<Vec<String>>,

        /// Update all containers
        #[arg(long)]
        all: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Logs {
            containers,
            tail,
            all,
        } => logs(containers, tail, all)?,
        Commands::Nuke => nuke()?,
        Commands::Restart { containers, all } => restart(containers, all)?,
        Commands::Update { containers, all } => update(containers, all)?,
    }

    Ok(())
}

/// Lists currently running docker containers
fn list_containers() -> anyhow::Result<Vec<String>> {
    if use_color() {
        color_println(Color::Green, "Listing docker containers...");
    } else {
        println!("Listing docker containers...")
    }

    // Use docker to list container_ids
    let container_ids = Command::new(DOCKER)
        .args(["ps", "-q"])
        .output()
        .context("Failed to list docker containers")?;

    // Turn Output into String
    let container_id_list = String::from_utf8(container_ids.stdout)
        .context("Failed to create string of container id's")?;

    // Parse/sanitize container ids and collecto into Vec
    let ids = container_id_list
        .split_whitespace()
        .map(String::from)
        .collect::<Vec<String>>();

    Ok(ids)
}

/// Force removes all docker containers provided in argument
fn kill_containers(container_ids: Vec<String>) -> anyhow::Result<()> {
    if use_color() {
        color_println(Color::Yellow, "Killing docker containers...");
    } else {
        println!("Killing docker containers...")
    }

    Command::new(DOCKER)
        .args(["rm", "-f"])
        .args(&container_ids)
        .status()
        .context("Failed to remove containers")?;

    Ok(())
}

/// Shows logs for specified containers
fn logs(containers: Option<Vec<String>>, tail: u32, all: bool) -> anyhow::Result<()> {
    let use_color = use_color();

    if all {
        let container_ids = list_containers()?;

        if container_ids.is_empty() {
            if use_color {
                color_println(Color::Red, "No containers running");
            } else {
                println!("No containers running");
            }
            return Ok(());
        }

        if use_color {
            color_println(
                Color::Cyan,
                &format!("Following logs for {} containers...", &container_ids.len()),
            );
        } else {
            println!("Following logs for {} containers...", &container_ids.len())
        }

        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let mut handles: Vec<std::thread::JoinHandle<()>> = vec![];

        for container in &container_ids {
            let tx = tx.clone();
            let container_id = container.clone();

            let handle = std::thread::spawn(move || {
                let container_name = match get_container_name(&container_id) {
                    Ok(name) => name,
                    Err(_) => container_id,
                };
                let mut logs_process = match Command::new(DOCKER)
                    .args([
                        "logs",
                        &container_name,
                        "--tail",
                        &tail.to_string(),
                        "--follow",
                    ])
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    Ok(proc) => proc,
                    Err(_) => {
                        let _ = tx.send(if use_color {
                            color_println_fmt(
                                Color::Red,
                                &format!("[ERROR] - Failed to log {container_name}"),
                            )
                        } else {
                            format!("[ERROR] - Failed to log {container_name}")
                        });
                        return;
                    }
                };

                if let Some(stdout) = logs_process.stdout.take() {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        if tx
                            .send(if use_color {
                                format!(
                                    "[{} | {}] {}",
                                    color_println_fmt(Color::Cyan, &get_timestamp()),
                                    color_println_fmt(Color::Green, &container_name),
                                    line
                                )
                            } else {
                                format!("[{} | {}] {}", &get_timestamp(), &container_name, line)
                            })
                            .is_err()
                        {
                            break; // Receiver closed
                        }
                    }
                }

                let _ = logs_process.kill();
                let _ = logs_process.wait();
            });

            handles.push(handle);
        }
        drop(tx);

        for log_line in rx {
            println!("{log_line}");
        }

        for handle in handles {
            let _ = handle.join();
        }
    } else if let Some(containers) = containers {
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
            let container_id = container.clone();

            let handle = std::thread::spawn(move || {
                let container_name = match get_container_name(&container_id) {
                    Ok(name) => name,
                    Err(_) => container_id,
                };

                let mut logs_process = match Command::new(DOCKER)
                    .args([
                        "logs",
                        &container_name,
                        "--tail",
                        &tail.to_string(),
                        "--follow",
                    ])
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    Ok(proc) => proc,
                    Err(_) => {
                        let _ = tx.send(if use_color {
                            color_println_fmt(
                                Color::Red,
                                &format!("[ERROR] - Failed to log {container_name}"),
                            )
                        } else {
                            format!("[ERROR] - Failed to log {container_name}")
                        });
                        return;
                    }
                };

                if let Some(stdout) = logs_process.stdout.take() {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        if tx
                            .send(if use_color {
                                format!(
                                    "[{} | {}] {}",
                                    color_println_fmt(Color::Cyan, &get_timestamp()),
                                    color_println_fmt(Color::Green, &container_name),
                                    line
                                )
                            } else {
                                format!("[{} | {}] {}", &get_timestamp(), &container_name, line)
                            })
                            .is_err()
                        {
                            break; // Receiver closed
                        }
                    }
                }

                let _ = logs_process.kill();
                let _ = logs_process.wait();
            });

            handles.push(handle);
        }
        drop(tx);

        for log_line in rx {
            println!("{log_line}");
        }

        for handle in handles {
            let _ = handle.join();
        }
    } else {
        anyhow::bail!("Must specify containers or use --all (-a)")
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
        .args(["compose", "-f", PATH_DSP_COMPOSE, "up", "-d"])
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
            PATH_DSP_COMPOSE,
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
                    color_println_fmt(Color::Magenta, DSP),
                    line
                );
            } else {
                println!("[{} | {}] {}", &get_timestamp(), DSP, line);
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
fn restart(containers: Option<Vec<String>>, all: bool) -> anyhow::Result<()> {
    let use_color = use_color();

    if all {
        let container_ids = list_containers()?;

        for container in &container_ids {
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
    } else if let Some(containers) = containers {
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
    } else {
        anyhow::bail!("Must specify containers or use --all (-a)")
    }

    Ok(())
}

/// Updates images of specified docker containers
fn update(containers: Option<Vec<String>>, all: bool) -> anyhow::Result<()> {
    let use_color = use_color();

    if all {
        let container_ids = list_containers()?;

        for container in &container_ids {
            update_container_by_name(container)?
        }

        if use_color {
            color_println(Color::Green, &format!("Restarting {DSP}"));
        } else {
            println!("Restarting {DSP}")
        }

        // containers updated, restart docker-stack-deploy to deploy new image
        Command::new(DOCKER)
            .args(["restart", DSP])
            .status()
            .context(format!("Failed to restart {DSP}"))?;
    } else if let Some(containers) = containers {
        for container in &containers {
            update_container_by_name(container)?;
        }
        if use_color {
            color_println(Color::Green, &format!("Restarting {DSP}"));
        } else {
            println!("Restarting {DSP}");
        }

        // containers updated, restart docker-stack-deploy to deploy new image
        Command::new(DOCKER)
            .args(["restart", DSP])
            .status()
            .context(format!("Failed to restart {DSP}"))?;
    } else {
        anyhow::bail!("Must specify containers or use --all (-a)")
    }

    Ok(())
}

fn use_color() -> bool {
    std::io::stdout().is_terminal()
}

/// Gets the current time on the system in readable format
fn get_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Gets the name of a docker container by the container_id passed as argument
fn get_container_name(container_id: &str) -> anyhow::Result<String> {
    // get container name by referencing id
    let output = Command::new(DOCKER)
        .args(["inspect", "--format", "{{.Name}}", container_id])
        .output()
        .context("Failed to inspect container")?;

    // parse output into clean String
    let name = String::from_utf8(output.stdout)
        .context("Failed to parse container name from output")?
        .trim()
        .trim_start_matches('/') // Docker names start with '/'
        .to_string();

    Ok(name)
}

/// Updates a container by the container_name provided as argument
fn update_container_by_name(container_name: &str) -> anyhow::Result<()> {
    // get container image string by referenciing the container_name
    let image_output = Command::new(DOCKER)
        .args(["inspect", "--format", "{{.Config.Image}}", container_name])
        .output()
        .context("Failed to inspect container")?;

    // parse output into clean String
    let image_name = String::from_utf8(image_output.stdout)
        .context("Failed to parse image name from output")?
        .trim()
        .to_string();

    if use_color() {
        color_println(
            Color::Cyan,
            &format!(
                "Pulling latest image for {}: {}",
                &container_name, &image_name
            ),
        );
    } else {
        println!(
            "Pulling latest image for {}: {}",
            &container_name, &image_name
        )
    }

    // pull new image for container
    Command::new(DOCKER)
        .args(["pull", &image_name])
        .status()
        .context(format!("Failed to pull image: {}", &image_name))?;

    Ok(())
}
