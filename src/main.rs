use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List { all, details } => powsup::list_ports(all, details),
        Command::Off => powsup::off(&get_port(cli)?),
        Command::On => powsup::on(&get_port(cli)?),
        Command::Powercycle { duration } => powsup::powercycle(&get_port(cli)?, duration),
        Command::Status => powsup::status(&get_port(cli)?),
    }
}

fn get_port(cli: Cli) -> Result<String> {
    if let Some(port) = cli.serial_port {
        Ok(port)
    } else {
        powsup::guess_port().with_context(|| "Failed to guess serial-port of power-supply.  Use option `--serial-port` to select one.  Try the command `powsup list --all` to get a list of all serial-ports.")
    }
}

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
    /// The serial port that the power supply is connected to.
    #[clap(short, long)]
    serial_port: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List serial ports where a power-supply might be connected to
    List {
        /// List all available serial ports
        #[clap(short, long)]
        all: bool,
        /// Print details about the serial ports
        #[clap(short, long)]
        details: bool,
    },
    /// Turn the output off
    Off,
    /// Turn the output on
    On,
    /// Turn the output off and after x seconds back on
    Powercycle {
        /// The duration in seconds that the output should be turned off
        #[clap(default_value_t = 3)]
        duration: u64,
    },
    /// Get the preset and the actual voltage and current values
    Status,
}
