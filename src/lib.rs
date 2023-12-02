use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serialport::{ClearBuffer, SerialPort, SerialPortInfo, SerialPortType};
use std::{collections::VecDeque, io, str::from_utf8, time, time::Duration};

pub fn list_ports(args: &ListArgs) -> Result<()> {
    let ports =
        serialport::available_ports().context("Failed to enumerate the available serial ports.")?;
    let predicate: fn(&SerialPortInfo) -> bool = if args.all { |_| true } else { is_powersupply };
    for p in ports.into_iter().filter(predicate) {
        println!("{}", p.port_name);
        if args.details {
            format!("{:#?}", p.port_type).lines().for_each(|a| {
                println!("    |{a}");
            });
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

pub fn interactive(powsup: &mut PowSup, args: &InteractiveArgs) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, powsup, args);

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

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    powsup: &mut PowSup,
    args: &InteractiveArgs,
) -> Result<()> {
    let mut last_tick = time::Instant::now();
    let mut last_powercycle: Option<time::Instant> = None;
    loop {
        if last_tick.elapsed() >= args.period {
            terminal.draw(|f| update_tui(f, powsup))?;
            last_tick = time::Instant::now();
        }
        if let Some(last_pc) = last_powercycle {
            if last_pc.elapsed() >= args.off_duration {
                powsup.on()?;
                last_powercycle = None;
            }
        }

        let timeout = args
            .period
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('p') => powsup.on()?,
                    KeyCode::Char('n') => powsup.off()?,
                    KeyCode::Char('c') => {
                        powsup.off()?;
                        last_powercycle = Some(time::Instant::now());
                    }
                    KeyCode::Char('q') => return Ok(()),
                    _other => (),
                }
            }
        }
    }
}

fn update_tui(f: &mut Frame, powsup: &mut PowSup) {
    let mut message: Vec<Line> = Vec::new();
    let (max_v, max_i) = match powsup.get_max() {
        Ok((a, b)) => (a, b),
        Err(err) => {
            message.push(Line::from(Span::styled(
                err.to_string(),
                Style::default().fg(Color::Red),
            )));
            (f32::NAN, f32::NAN)
        }
    };
    let (preset_v, preset_i) = match powsup.get_preset() {
        Ok((a, b)) => (a, b),
        Err(err) => {
            message.push(Line::from(Span::styled(
                err.to_string(),
                Style::default().fg(Color::Red),
            )));
            (f32::NAN, f32::NAN)
        }
    };
    let (display_v, display_i, display_mode) = match powsup.get_display() {
        Ok((a, b, c)) => (a, b, if c { "CC" } else { "CV" }),
        Err(err) => {
            message.push(Line::from(Span::styled(
                err.to_string(),
                Style::default().fg(Color::Red),
            )));
            (f32::NAN, f32::NAN, "--")
        }
    };
    let ppanes = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(6),
                Constraint::Min(10),
                Constraint::Length(5),
            ]
            .as_ref(),
        )
        .split(f.size());
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
        .split(ppanes[0]);

    let block = Block::default()
        .title(if let Some(s) = powsup.port.name() {
            format!(" {} ", s)
        } else {
            String::from(" <unknown port> ")
        })
        .borders(Borders::ALL);
    let text = vec![
        Line::from("        Voltage   Current    "),
        Line::from(format!("Maximum: {:5.2} V   {:5.2} A    ", max_v, max_i)),
        Line::from(format!(
            "Preset:  {:5.2} V   {:5.2} A    ",
            preset_v, preset_i
        )),
        Line::from(format!(
            "Actual:  {:5.2} V   {:5.2} A  {}",
            display_v, display_i, display_mode
        )),
    ];
    let paragraph = Paragraph::new(text.clone())
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(paragraph, panes[0]);

    // right side
    let block = Block::default()
        .title(" Key bindings ")
        .borders(Borders::ALL);
    let text = vec![
        Line::from("p => Power on   "),
        Line::from("n => Power off  "),
        Line::from("c => Power cycle"),
        Line::from("q => Quit       "),
    ];
    let paragraph = Paragraph::new(text.clone())
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(paragraph, panes[1]);

    // middle block
    let data: Vec<(f64, f64)> = std::iter::zip(1..300, &powsup.trend)
        .map(|(x, (_, i))| (x.into(), (*i).into()))
        .collect();
    let datasets = vec![Dataset::default()
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .data(&data)];
    let chart = Chart::new(datasets)
        .block(Block::default())
        .x_axis(Axis::default().bounds([1.0, 300.0]))
        .y_axis(
            Axis::default()
                .title("A")
                .labels(vec![
                    Span::raw("0"),
                    Span::raw(format!("{}", preset_i / 2.0)),
                    Span::raw(format!("{}", preset_i)),
                ])
                .bounds([0.0, preset_i.into()]),
        );
    f.render_widget(chart, ppanes[1]);

    // lower block
    let block = Block::default().title(" Messages ").borders(Borders::ALL);
    let paragraph = Paragraph::new(message.clone()).block(block);
    f.render_widget(paragraph, ppanes[2]);
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
    trend: VecDeque<(f32, f32)>,
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
            trend: VecDeque::with_capacity(300),
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

    pub fn powercycle(&mut self, duration: Duration) -> Result<()> {
        self.write("SOUT1\r")?;
        self.expect_ok()?;
        std::thread::sleep(duration);
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
        while self.trend.len() >= 300 {
            self.trend.pop_front();
        }
        self.trend.push_back((v, c));
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

    pub fn status(&mut self, brief: bool) -> Result<()> {
        if !brief {
            let (v, i) = self.get_max()?;
            println!("Maximum: {:5.2} V  {:5.2} A", v, i);
            let (v, i) = self.get_preset()?;
            println!("Preset:  {:5.2} V  {:5.2} A", v, i);
        }
        let (v, i, cc) = self.get_display()?;
        println!(
            "Display: {:5.2} V  {:5.2} A  {}",
            v,
            i,
            if cc { "CC" } else { "CV" }
        );
        Ok(())
    }
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// List all available serial ports
    #[clap(short, long)]
    all: bool,
    /// Print details about the serial ports
    #[clap(short, long)]
    details: bool,
}

#[derive(Debug, Args)]
pub struct InteractiveArgs {
    /// The pause between refreshs in milliseconds
    #[clap(short, long, default_value = "600", value_parser = ms_parser)]
    period: Duration,
    /// The duration in milliseconds that the output should be turned off during powercycle
    #[clap(short, long, default_value = "3000", value_parser = ms_parser)]
    off_duration: Duration,
}

impl Default for InteractiveArgs {
    fn default() -> InteractiveArgs {
        InteractiveArgs {
            period: Duration::from_millis(600),
            off_duration: Duration::from_millis(3000),
        }
    }
}

pub fn ms_parser(ms: &str) -> std::result::Result<Duration, std::num::ParseIntError> {
    ms.parse().map(Duration::from_millis)
}
