use crate::printer::{color_println, color_println_fmt, Color};
use crate::DOCKER;
use anyhow::Context;
use chrono::Local;
use std::io::{BufRead, BufReader, IsTerminal};
use std::process::{Command, Stdio};
use std::sync::Arc;

pub fn use_color() -> bool {
    std::io::stdout().is_terminal()
}

/// Gets the current time on the system in readable format
pub fn get_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Lists currently running docker containers
pub fn list_containers() -> anyhow::Result<Vec<String>> {
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
pub fn kill_containers(container_ids: Vec<String>) -> anyhow::Result<()> {
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

/// Gets container names from a given stack
pub fn get_containers_from_stack(stack: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new(DOCKER)
        .args([
            "ps",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={}", &stack),
        ])
        .output()
        .context(format!("Failed to containers in stack: {}", &stack))?;

    let container_ids =
        String::from_utf8(output.stdout).expect("Failed to parse container name from output");

    let container_ids_vec = container_ids.split_whitespace().map(String::from);

    let containers = container_ids_vec
        .filter_map(|id| get_container_name(&id).ok())
        .collect();

    Ok(containers)
}

/// Gets the name of a docker container by the container_id passed as argument
pub fn get_container_name(container_id: &str) -> anyhow::Result<String> {
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
pub fn update_container_by_name(container_name: &str) -> anyhow::Result<u8> {
    let mut is_updated: u8 = 0;
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
            &format!("Pulling image for {}: {}", &container_name, &image_name),
        );
    } else {
        println!("Pulling image for {}: {}", &container_name, &image_name)
    }

    // pull new image for container
    let mut logs_process = Command::new(DOCKER)
        .args(["pull", &image_name])
        .stdout(Stdio::piped())
        .spawn()
        .context(format!("Failed to pull image: {}", &image_name))?;

    if let Some(stdout) = logs_process.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            println!("{line}");
            if line.contains("Status: Downloaded newer image") {
                is_updated = 1
            }
        }
    }

    let _ = logs_process.kill();
    let _ = logs_process.wait();

    Ok(is_updated)
}

pub fn spawn_container_logger(
    container: &str,
    is_container_id: bool,
    use_color: bool,
    tail: u32,
    tx: std::sync::mpsc::Sender<String>,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    let container_identifier = Arc::new(container.to_string());

    let handle = std::thread::spawn(move || {
        let container_name = if is_container_id {
            match get_container_name(&container_identifier) {
                Ok(name) => Arc::new(name),
                Err(_) => Arc::clone(&container_identifier),
            }
        } else {
            Arc::clone(&container_identifier)
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
            .stderr(Stdio::piped())
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

        let mut handles: Vec<std::thread::JoinHandle<()>> = vec![];

        // handle stdout
        if let Some(stdout) = logs_process.stdout.take() {
            let tx_stdout = tx.clone();
            let container_name_stdout = Arc::clone(&container_name);
            let handle_stdout = std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if tx_stdout
                        .send(if use_color {
                            format!(
                                "[{} | {}] {}",
                                color_println_fmt(Color::Cyan, &get_timestamp()),
                                color_println_fmt(Color::Green, &container_name_stdout),
                                line
                            )
                        } else {
                            format!(
                                "[{} | {}] {}",
                                &get_timestamp(),
                                &container_name_stdout,
                                line
                            )
                        })
                        .is_err()
                    {
                        break; // Receiver closed
                    }
                }
            });

            handles.push(handle_stdout);
        }

        // handle stderr
        if let Some(stderr) = logs_process.stderr.take() {
            let tx_stderr = tx.clone();
            let container_name_stderr = Arc::clone(&container_name);
            let handle_stderr = std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if tx_stderr
                        .send(if use_color {
                            format!(
                                "[{} | {}] {}",
                                color_println_fmt(Color::Cyan, &get_timestamp()),
                                color_println_fmt(Color::Green, &container_name_stderr),
                                line
                            )
                        } else {
                            format!(
                                "[{} | {}] {}",
                                &get_timestamp(),
                                &container_name_stderr,
                                line
                            )
                        })
                        .is_err()
                    {
                        break; // Receiver closed
                    }
                }
            });

            handles.push(handle_stderr);
        }

        for handle in handles {
            let _ = handle.join();
        }

        let _ = logs_process.kill();
        let _ = logs_process.wait();
    });

    Ok(handle)
}
