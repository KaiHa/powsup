use anyhow::{bail, Context, Result};
use serialport::{ClearBuffer, SerialPort, SerialPortInfo, SerialPortType};
use std::{str::from_utf8, time::Duration};

pub fn list_ports(all: bool, details: bool) -> Result<()> {
    let ports = serialport::available_ports()
        .with_context(|| "Failed to enumerate the available serial ports.")?;
    let predicate: fn(&SerialPortInfo) -> bool = if all { |_| true } else { is_powersupply };
    for p in ports.into_iter().filter(predicate) {
        if details {
            let port_type = format!("{:#?}", p.port_type)
                .lines()
                .map(|x| format!("    {}\n", x))
                .collect::<String>();
            println!("{}:\n{}", p.port_name, port_type);
        } else {
            println!("{}", p.port_name);
        }
    }
    Ok(())
}

pub fn guess_port() -> Result<String> {
    let ports: Vec<SerialPortInfo> = serialport::available_ports()
        .with_context(|| "Failed to enumerate the available serial ports.")?
        .into_iter()
        .filter(is_powersupply)
        .collect();
    if ports.len() > 1 {
        bail!("Found multiple serial ports that might be connected to a power-supply.",)
    } else if let Some(p) = ports.first() {
        Ok(p.port_name.to_string())
    } else {
        bail!("Found no serial port that might be connected to a power-supply. ")
    }
}

pub fn off(port: &str) -> Result<()> {
    let mut powsup = PowSup::new(port)?;
    powsup.write("SOUT1\r")?;
    powsup.expect_ok()
}

pub fn on(port: &str) -> Result<()> {
    let mut powsup = PowSup::new(port)?;
    powsup.write("SOUT0\r")?;
    powsup.expect_ok()
}

pub fn powercycle(port: &str, duration: u64) -> Result<()> {
    let mut powsup = PowSup::new(port)?;
    powsup.write("SOUT1\r")?;
    powsup.expect_ok()?;
    std::thread::sleep(Duration::from_secs(duration));
    powsup.write("SOUT0\r")?;
    powsup.expect_ok()
}

pub fn status(port: &str) -> Result<()> {
    let mut powsup = PowSup::new(port)?;
    for (cmd, label) in &[
        ("GMAX\r", "Maximum"),
        ("GETS\r", "Preset"),
        ("GETD\r", "Display"),
    ] {
        powsup.write(cmd)?;
        let reply = powsup.read()?;
        // Print common part
        if reply.len() >= 10 {
            print!("{:9} {}.", label, &reply[0..2]);
        }
        // Print remaining data
        if reply.len() == 10 {
            println!("{:2}V  {}.{:3}A", &reply[2..3], &reply[3..5], &reply[5..6]);
        } else if reply.len() == 11 {
            println!("{:2}V  {}.{:3}A", &reply[2..3], &reply[3..5], &reply[5..7]);
        } else if reply.len() == 13 {
            println!("{:2}V  {}.{:3}A", &reply[2..4], &reply[4..6], &reply[6..9]);
        } else {
            bail!(
                "Unexpected length {} of reply from power-supply: {:?}",
                reply.len(),
                &reply
            )
        };
    }
    Ok(())
}

fn is_powersupply(SerialPortInfo { port_type, .. }: &SerialPortInfo) -> bool {
    if let SerialPortType::UsbPort(info) = port_type {
        if let Some(manufacturer) = &info.manufacturer {
            manufacturer.contains("Silicon Labs")
        } else {
            false
        }
    } else {
        false
    }
}

struct PowSup {
    port: Box<dyn SerialPort>,
}

impl PowSup {
    fn new(port: &str) -> Result<PowSup> {
        log::trace!("opening port");
        let port = serialport::new(port, 9600)
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::One)
            .parity(serialport::Parity::None)
            .flow_control(serialport::FlowControl::None)
            // TODO figure out what a good timeout could be
            .timeout(Duration::from_millis(1000))
            .open()
            .with_context(|| format!("Failed to open the serial port \"{}\"", port))?;
        port.clear(ClearBuffer::All)?;
        Ok(PowSup { port })
    }

    fn write(&mut self, s: &str) -> Result<()> {
        log::debug!("write: sending {:?}", s);
        self.port
            .write_all(s.as_bytes())
            .with_context(|| "Write to serial port failed.")
    }

    fn read(&mut self) -> Result<String> {
        let mut s = String::new();
        let mut is_incomplete = true;
        for i in 1..20 {
            let mut buf: Vec<u8> = vec![0; 32];
            std::thread::sleep(Duration::from_millis(20));
            self.port
                .read(buf.as_mut_slice())
                .with_context(|| "Read from serial port failed.")?;
            log::trace!("read: #{} got {:?}", &i, &buf);
            s.push_str(from_utf8(
                &buf.into_iter().take_while(|&x| x != 0).collect::<Vec<u8>>(),
            )?);
            if s.ends_with("OK\r") {
                is_incomplete = false;
                break;
            }
        }
        log::debug!("read: got {:?}", &s);
        if is_incomplete {
            bail!("Incomplete reply from power-supply: {:?}", &s)
        };
        Ok(s)
    }

    /// Read the return value from the power-supply and return an error if the value is not "OK\r"
    fn expect_ok(&mut self) -> Result<()> {
        let result = self.read()?;
        if result == "OK\r" {
            Ok(())
        } else {
            bail!(
                "Got an unexpected reply from the power-supply: {:?}",
                &result
            )
        }
    }
}
