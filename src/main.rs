use clap::{Arg, Command};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter};
use std::path::Path;
use std::process::Command as ShellCommand;
use serde::{Deserialize, Serialize};

const STORAGE_FILE: &str = "~/.devbox_storage.json";

#[derive(Serialize, Deserialize, Debug)]
struct DevboxStorage {
    containers: HashMap<String, Vec<String>>, // Maps SSH names to container lists
}

fn main() {
    let matches = Command::new("devbox")
        .version("1.0")
        .about("Development tool for managing container connections")
        .subcommand(
            Command::new("init")
                .about("Initialize devbox with available containers from SSH server")
                .arg(
                    Arg::new("sshname")
                        .required(true)
                        .help("SSH name for the remote machine"),
                ),
        )
        .subcommand(
            Command::new("nvim")
                .about("Connect to nvim in a specified container on a specified SSH server")
                .arg(
                    Arg::new("sshname")
                        .required(true)
                        .help("SSH name for the remote machine"),
                )
                .arg(
                    Arg::new("container")
                        .required(true)
                        .help("Specify which container to connect to"),
                ),
        )
        .subcommand(
            Command::new("list")
                .about("List stored container names for all SSH hosts"),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("init", sub_m)) => {
            let sshname = sub_m.get_one::<String>("sshname").unwrap();
            match initialize_containers(sshname) {
                Ok(_) => println!("Initialized containers for SSH name: {}", sshname),
                Err(e) => eprintln!("Error initializing containers: {}", e),
            }
        }
        Some(("nvim", sub_m)) => {
            let sshname = sub_m.get_one::<String>("sshname").unwrap();
            let container = sub_m.get_one::<String>("container").unwrap();
            let ssh_command = format!(
                "ssh {} -t 'docker exec -it {} bash -c \"tmux attach-session -t nvim || tmux new-session -s nvim\"'",
                sshname, container
            );
            execute_command(&ssh_command, &format!("Neovim in container '{}'", container));
        }
        Some(("list", _)) => {
            match load_storage() {
                Ok(storage) => {
                    println!("Stored containers by SSH name:");
                    for (sshname, containers) in storage.containers.iter() {
                        println!("- {}: {:?}", sshname, containers);
                    }
                }
                Err(e) => eprintln!("Error loading storage: {}", e),
            }
        }
        _ => eprintln!("No valid subcommand was provided"),
    }
}

fn initialize_containers(sshname: &str) -> io::Result<()> {
    // Run `docker ps -a` over SSH to get container names
    let output = ShellCommand::new("ssh")
        .arg(sshname)
        .arg("docker ps -a --format {{.Names}}")
        .output()?;

    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to fetch container names over SSH",
        ));
    }

    // Parse the container names from the command output
    let container_names: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .collect();

    // Load existing storage and add the new SSH name and containers
    let mut storage = load_storage().unwrap_or_else(|_| DevboxStorage {
        containers: HashMap::new(),
    });
    storage.containers.insert(sshname.to_string(), container_names);

    // Save updated storage
    save_storage(&storage)
}

fn load_storage() -> io::Result<DevboxStorage> {
    let path = shellexpand::tilde(STORAGE_FILE).to_string();
    if !Path::new(&path).exists() {
        return Ok(DevboxStorage {
            containers: HashMap::new(),
        });
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn save_storage(storage: &DevboxStorage) -> io::Result<()> {
    let path = shellexpand::tilde(STORAGE_FILE).to_string();
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, storage).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn execute_command(command: &str, context: &str) {
    let status = ShellCommand::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .expect("Failed to execute command");

    if status.success() {
        println!("Successfully executed {} command.", context);
    } else {
        eprintln!("Failed to execute {} command.", context);
    }
}

