use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, WarnLevel};
use simple_logger::SimpleLogger;

fn main() -> Result<()> {
    let cli = Cli::parse();
    SimpleLogger::new()
        .with_level(cli.verbose.log_level_filter())
        .init()?;
    match cli.command {
        Command::List { all, details } => powsup::list_ports(all, details),
        Command::Off => get_powsup(cli)?.off(),
        Command::On => get_powsup(cli)?.on(),
        Command::Powercycle { duration } => get_powsup(cli)?.powercycle(duration),
        Command::Status { brief } => powsup::status(&mut get_powsup(cli)?, brief),
        Command::Interactive => powsup::interactive(&mut get_powsup(cli)?),
    }
}

fn get_powsup(cli: Cli) -> Result<powsup::PowSup> {
    let port = if let Some(port) = cli.serial_port {
        Ok(port)
    } else {
        powsup::guess_port().context("Failed to guess serial-port of power-supply.  Use option `--serial-port` to select one.  Try the command `powsup list --all` to get a list of all serial-ports.")
    };
    powsup::PowSup::new(&port?)
}

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
    /// The serial port that the power supply is connected to.
    #[clap(short, long)]
    serial_port: Option<String>,
    #[clap(flatten)]
    verbose: Verbosity<WarnLevel>,
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
    Status {
        /// Only show display value
        #[clap(short, long)]
        brief: bool,
    },
    /// Run in interactive mode (press 'q' to exit)
    Interactive,
}
