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
        Command::List { ref args } => powsup::list_ports(args),
        Command::Off => get_powsup(&cli)?.off(),
        Command::On => get_powsup(&cli)?.on(),
        Command::Powercycle { off_duration } => get_powsup(&cli)?.powercycle(off_duration),
        Command::Status { brief } => get_powsup(&cli)?.status(brief),
        Command::Interactive { ref args } => powsup::interactive(&mut get_powsup(&cli)?, args),
    }
}

fn get_powsup(cli: &Cli) -> Result<powsup::PowSup> {
    let port = if let Some(port) = cli.serial_port.clone() {
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
        #[clap(flatten)]
        args: powsup::ListArgs,
    },
    /// Turn the output off
    Off,
    /// Turn the output on
    On,
    /// Turn the output off and after x seconds back on
    Powercycle {
        /// The duration in seconds that the output should be turned off
        #[clap(short, long, default_value_t = 3)]
        off_duration: u64,
    },
    /// Get the preset and the actual voltage and current values
    Status {
        /// Only show display value
        #[clap(short, long)]
        brief: bool,
    },
    /// Run in interactive mode (press 'q' to exit)
    Interactive {
        #[clap(flatten)]
        args: powsup::InteractiveArgs,
    },
}
