use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

const COMPOSE_PATH: &str = "/var/lib/docker-stack-deploy/compose.yml";

fn main() {
    // get list of currently running docker containers by id
    let container_ids = Command::new("docker")
        .args(["ps", "-q"])
        .output()
        .expect("Failed to list docker containers");

    // if docker containers are running, kill them
    if !container_ids.stdout.is_empty() {
        let container_list = String::from_utf8(container_ids.stdout).unwrap();
        let ids: Vec<&str> = container_list.split_whitespace().collect();

        println!("Killing docker containers...");

        Command::new("docker")
            .args(["rm", "-f"])
            .args(ids)
            .status()
            .expect("Failed to remove containers");
    }

    println!("Starting docker-stack-deploy...");

    // run docker-stack-deploy
    Command::new("docker")
        .args(["compose", "-f", COMPOSE_PATH, "up", "-d"])
        .status()
        .expect("Failed to start docker-stack-deploy");

    println!("Following logs until all containers deployed...");

    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // follow docker-stack-deploy logs until first update check has happened
    let mut logs_process = Command::new("docker")
        .args([
            "compose",
            "-f",
            COMPOSE_PATH,
            "logs",
            "-f",
            "--since",
            &start_time.to_string(),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start following logs");

    if let Some(stdout) = logs_process.stdout.take() {
        let reader = BufReader::new(stdout);
        for (i, line) in reader.lines().map_while(Result::ok).enumerate() {
            println!("{line}");
            if line.contains("Already up to date") && i > 0 {
                // first update check has happened after deployment
                break;
            }
        }
    }

    let _ = logs_process.kill();
    let _ = logs_process.wait();
}
