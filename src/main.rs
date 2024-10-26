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
        .subcommand(
            Command::new("fp")
                .about("Forward port from a specific container to the calling host")
                .arg(
                    Arg::new("sshname")
                        .required(true)
                        .help("SSH name for the remote machine"),
                )
                .arg(
                    Arg::new("container")
                        .required(true)
                        .help("Specify which container to forward the port from"),
                )
                .arg(
                    Arg::new("port")
                        .required(true)
                        .help("Specify the port to forward from the container"),
                ),
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
        Some(("fp", sub_m)) => {
            let sshname = sub_m.get_one::<String>("sshname").unwrap();
            let container = sub_m.get_one::<String>("container").unwrap();
            let port = sub_m.get_one::<String>("port").unwrap();

            match fetch_container_ip(sshname, container) {
                Ok(container_ip) => {
                    let ssh_command = format!(
                        "ssh -L {0}:{1}:{0} {2} -N",
                        port, container_ip, sshname
                    );
                    execute_command(&ssh_command, &format!("Port forwarding for container '{}'", container));
                }
                Err(e) => eprintln!("Failed to fetch IP for container '{}': {}", container, e),
            }
        }
        _ => eprintln!("No valid subcommand was provided"),
    }
}

fn fetch_container_ip(sshname: &str, container: &str) -> io::Result<String> {
    // Step 1: Fetch the network name(s) for the container
    let network_output = ShellCommand::new("ssh")
        .arg(sshname)
        .arg(format!(
            "docker inspect {} | jq -r '.[0].NetworkSettings.Networks | keys[0]'",
            container
        ))
        .output()?;

    if !network_output.status.success() {
        eprintln!(
            "Error fetching network name: {}",
            String::from_utf8_lossy(&network_output.stderr)
        );
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to fetch container network name over SSH",
        ));
    }

    let network_name = String::from_utf8_lossy(&network_output.stdout).trim().to_string();
    if network_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "No network found for the specified container",
        ));
    }

    // Step 2: Fetch the container IP address on the determined network
    let ip_output = ShellCommand::new("ssh")
        .arg(sshname)
        .arg(format!(
            "docker inspect {} | jq -r '.[0].NetworkSettings.Networks[\"{}\"].IPAddress'",
            container, network_name
        ))
        .output()?;

    if !ip_output.status.success() {
        eprintln!(
            "Error fetching IP address: {}",
            String::from_utf8_lossy(&ip_output.stderr)
        );
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to fetch container IP address over SSH",
        ));
    }

    let ip_address = String::from_utf8_lossy(&ip_output.stdout).trim().to_string();
    if ip_address.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "No IP address assigned to the container on the detected network",
        ))
    } else {
        Ok(ip_address)
    }
}

fn initialize_containers(sshname: &str) -> io::Result<()> {
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

    let container_names: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .collect();

    let mut storage = load_storage().unwrap_or_else(|_| DevboxStorage {
        containers: HashMap::new(),
    });
    storage.containers.insert(sshname.to_string(), container_names);

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

