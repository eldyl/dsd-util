use clap::{Parser, Subcommand};
use dsd_util::commands::{init, logs, nuke, restart, stats, update};

const DEFAULT_ARG_PROJECT_DIR: &str = "/var/lib/docker-stack-deploy";
const DEFAULT_ARG_TAIL: &str = "100";

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
        #[arg(short, long, default_value = DEFAULT_ARG_PROJECT_DIR)]
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
        #[arg(short, long)]
        stacks: Option<Vec<String>>,

        /// Set the number of lines to show from end of logs
        #[arg(short, long, default_value = DEFAULT_ARG_TAIL)]
        tail: u32,

        /// View logs for all containers
        #[arg(short, long)]
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
        #[arg(short, long)]
        stacks: Option<Vec<String>>,

        /// Restart all containers
        #[arg(short, long)]
        all: bool,
    },

    /// View basic stats for docker containers
    Stats {
        /// View stats for specified containers
        containers: Option<Vec<String>>,

        /// View stats for specified stacks
        #[arg(short, long)]
        stacks: Option<Vec<String>>,

        /// View stats for all containers
        #[arg(short, long)]
        all: bool,
    },

    /// Update container images
    Update {
        /// Update specified containers
        containers: Option<Vec<String>>,

        /// Update specified stacks
        #[arg(short, long)]
        stacks: Option<Vec<String>>,

        /// Update all containers
        #[arg(short, long)]
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
