use anyhow::{bail, Context, Result};
use serialport::{SerialPort, SerialPortInfo, SerialPortType};
use std::time::Duration;

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
        let (current, ack) = if reply.len() == 10 {
            // HCS-3100, 3150, 3200, 3202
            (&reply[5..6], &reply[7..10])
        } else if reply.len() == 11 {
            // HCS-3102, 3104, 3204
            (&reply[5..7], &reply[8..11])
        } else {
            bail!("Unexpected length of reply from power-supply: {:?}", &reply)
        };
        if ack != "OK\r" {
            bail!("Unexpected reply from power-supply: {:?}", &reply)
        };
        println!(
            "{:9} {}.{}V  {}.{}A",
            label,
            &reply[0..2],
            &reply[2..3],
            &reply[3..5],
            current
        );
    }
    Ok(())
}

fn is_powersupply(SerialPortInfo { port_type, .. }: &SerialPortInfo) -> bool {
    if let SerialPortType::UsbPort(info) = port_type {
        if let Some(manufacturer) = &info.manufacturer {
            // TODO verify this
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
        let port = serialport::new(port, 9600)
            // TODO figure out what a good timeout could be
            .timeout(Duration::from_millis(1000))
            .open()
            .with_context(|| format!("Failed to open the serial port \"{}\"", port))?;
        Ok(PowSup { port })
    }

    fn write(&mut self, s: &str) -> Result<()> {
        self.port
            .write_all(s.as_bytes())
            .with_context(|| "Write to serial port failed.")
    }

    fn read(&mut self) -> Result<String> {
        let mut buf: Vec<u8> = vec![0; 32];
        self.port
            .read(buf.as_mut_slice())
            .with_context(|| "Read from serial port failed.")?;
        Ok(std::str::from_utf8(&buf)?.to_string())
    }

    /// Read the return value from the power-supply and raise return an error if the value is not "OK\r"
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
