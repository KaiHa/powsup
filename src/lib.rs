use anyhow::{anyhow, bail, Context, Result};
use serialport::{ClearBuffer, SerialPort, SerialPortInfo, SerialPortType};
use std::{io, str::from_utf8, time::Duration};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

pub fn list_ports(all: bool, details: bool) -> Result<()> {
    let ports =
        serialport::available_ports().context("Failed to enumerate the available serial ports.")?;
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
        .context("Failed to enumerate the available serial ports.")?
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

pub fn status(powsup: &mut PowSup, brief: bool) -> Result<()> {
    if !brief {
        let (v, i) = powsup.get_max()?;
        println!("Maximum: {:5.2} V  {:5.2} A", v, i);
        let (v, i) = powsup.get_preset()?;
        println!("Preset:  {:5.2} V  {:5.2} A", v, i);
    }
    let (v, i, cc) = powsup.get_display()?;
    println!(
        "Display: {:5.2} V  {:5.2} A  {}",
        v,
        i,
        if cc { "CC" } else { "CV" }
    );
    Ok(())
}

pub fn interactive(powsup: &mut PowSup) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, powsup);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        Err(anyhow!(err))
    } else {
        Ok(())
    }
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, powsup: &mut PowSup) -> Result<()> {
    loop {
        terminal.draw(|f| update_tui(f, powsup))?;

        if event::poll(Duration::from_millis(600))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('p') => powsup.on()?,
                    KeyCode::Char('n') => powsup.off()?,
                    KeyCode::Char('c') => powsup.powercycle(3)?,
                    KeyCode::Char('q') => return Ok(()),
                    _other => (),
                }
            }
        }
    }
}

fn update_tui<B: Backend>(f: &mut Frame<B>, powsup: &mut PowSup) {
    let mut message: Vec<Spans> = Vec::new();
    let (max_v, max_i) = match powsup.get_max() {
        Ok((a, b)) => (a, b),
        Err(err) => {
            message.push(Spans::from(Span::styled(
                err.to_string(),
                Style::default().fg(Color::Red),
            )));
            (f32::NAN, f32::NAN)
        }
    };
    let (preset_v, preset_i) = match powsup.get_preset() {
        Ok((a, b)) => (a, b),
        Err(err) => {
            message.push(Spans::from(Span::styled(
                err.to_string(),
                Style::default().fg(Color::Red),
            )));
            (f32::NAN, f32::NAN)
        }
    };
    let (display_v, display_i, display_mode) = match powsup.get_display() {
        Ok((a, b, c)) => (a, b, if c { "CC" } else { "CV" }),
        Err(err) => {
            message.push(Spans::from(Span::styled(
                err.to_string(),
                Style::default().fg(Color::Red),
            )));
            (f32::NAN, f32::NAN, "--")
        }
    };
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
        .split(f.size());

    let block = Block::default()
        .title(if let Some(s) = powsup.port.name() {
            format!(" {} ", s)
        } else {
            String::from(" <unknown port> ")
        })
        .borders(Borders::ALL);
    let mut text = vec![
        Spans::from(""),
        Spans::from("        Voltage   Current    "),
        Spans::from(format!("Maximum: {:5.2} V   {:5.2} A    ", max_v, max_i)),
        Spans::from(format!(
            "Preset:  {:5.2} V   {:5.2} A    ",
            preset_v, preset_i
        )),
        Spans::from(format!(
            "Actual:  {:5.2} V   {:5.2} A  {}",
            display_v, display_i, display_mode
        )),
        Spans::from(""),
    ];
    text.append(&mut message);
    let paragraph = Paragraph::new(text.clone())
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(paragraph, panes[0]);

    // right side
    let block = Block::default()
        .title(" Key bindings ")
        .borders(Borders::ALL);
    let text = vec![
        Spans::from(""),
        Spans::from("p => Power on   "),
        Spans::from("n => Power off  "),
        Spans::from("c => Power cycle"),
        Spans::from("q => Quit       "),
    ];
    let paragraph = Paragraph::new(text.clone())
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(paragraph, panes[1]);
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

pub struct PowSup {
    port: Box<dyn SerialPort>,
    cached_max: Option<(f32, f32)>,
}

impl PowSup {
    pub fn new(port: &str) -> Result<PowSup> {
        log::trace!("opening port");
        let port = serialport::new(port, 9600)
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::One)
            .parity(serialport::Parity::None)
            .flow_control(serialport::FlowControl::None)
            .timeout(Duration::from_millis(20))
            .open()
            .with_context(|| format!("Failed to open the serial port \"{}\"", port))?;
        port.clear(ClearBuffer::All)?;
        Ok(PowSup {
            port,
            cached_max: Option::None,
        })
    }

    fn write(&mut self, s: &str) -> Result<()> {
        log::debug!("write: sending {:?}", s);
        self.port
            .write_all(s.as_bytes())
            .context("Write to serial port failed.")
    }

    fn read(&mut self) -> Result<String> {
        let mut s = String::new();
        let mut is_incomplete = true;
        for i in 1..20 {
            let mut buf: Vec<u8> = vec![0; 32];
            self.port
                .read(buf.as_mut_slice())
                .context("Read from serial port failed.")?;
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

    pub fn off(&mut self) -> Result<()> {
        self.write("SOUT1\r")?;
        self.expect_ok()
    }

    pub fn on(&mut self) -> Result<()> {
        self.write("SOUT0\r")?;
        self.expect_ok()
    }

    pub fn powercycle(&mut self, duration: u64) -> Result<()> {
        self.write("SOUT1\r")?;
        self.expect_ok()?;
        std::thread::sleep(Duration::from_secs(duration));
        self.write("SOUT0\r")?;
        self.expect_ok()
    }

    pub fn get_display(&mut self) -> Result<(f32, f32, bool)> {
        self.write("GETD\r")?;
        let reply = self.read()?;
        if reply.len() != 13 || &reply[10..] != "OK\r" {
            bail!(
                "Got an unexpected GETD reply from the power-supply: {:?}",
                &reply
            );
        }
        let v = format!("{}.{}", &reply[0..2], &reply[2..4])
            .parse::<f32>()
            .context("Failed to parse voltage from reply")?;
        let c = format!("{}.{}", &reply[4..6], &reply[6..8])
            .parse::<f32>()
            .context("Failed to parse current from reply")?;
        let cc = match &reply[8..9] {
            "0" => false,
            "1" => true,
            _other => bail!("Failed to parse const-current mode from reply"),
        };
        Ok((v, c, cc))
    }

    pub fn get_preset(&mut self) -> Result<(f32, f32)> {
        self.write("GETS\r")?;
        let reply = self.read()?;
        if reply.len() != 10 || &reply[7..] != "OK\r" {
            bail!(
                "Got an unexpected GETS reply from the power-supply: {:?}",
                &reply
            );
        }
        let v = format!("{}.{}", &reply[0..2], &reply[2..3])
            .parse::<f32>()
            .context("Failed to parse voltage from reply")?;
        let c = format!("{}.{}", &reply[3..5], &reply[5..6])
            .parse::<f32>()
            .context("Failed to parse current from reply")?;
        Ok((v, c))
    }

    pub fn get_max(&mut self) -> Result<(f32, f32)> {
        if let Some(max) = self.cached_max {
            Ok(max)
        } else {
            self.write("GMAX\r")?;
            let reply = self.read()?;
            if reply.len() != 10 || &reply[7..] != "OK\r" {
                bail!(
                    "Got an unexpected GMAX reply from the power-supply: {:?}",
                    &reply
                );
            }
            let v = format!("{}.{}", &reply[0..2], &reply[2..3])
                .parse::<f32>()
                .context("Failed to parse voltage from reply")?;
            let c = format!("{}.{}", &reply[3..5], &reply[5..6])
                .parse::<f32>()
                .context("Failed to parse current from reply")?;
            self.cached_max = Some((v, c));
            Ok((v, c))
        }
    }
}
