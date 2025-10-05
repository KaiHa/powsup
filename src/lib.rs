use anyhow::{bail, Context, Error, Result};
use circular_buffer::CircularBuffer;
use clap::Args;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use derive_more::{From, Into};
use std::fmt;
use ratatui::prelude::*;
use ratatui::widgets::*;
use serialport::{ClearBuffer, SerialPort, SerialPortInfo, SerialPortType};
use std::{io, str::from_utf8, time, time::Duration};

pub fn list_ports(args: &ListArgs) -> Result<()> {
    let ports =
        serialport::available_ports().context("Failed to enumerate the available serial ports.")?;
    let predicate: fn(&SerialPortInfo) -> bool = if args.list_all {
        |_| true
    } else {
        is_powersupply
    };
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
                    KeyCode::Char('j') => powsup.y_max_offset -= 1.0,
                    KeyCode::Char('k') => powsup.y_max_offset += 1.0,
                    KeyCode::Char('q') => return Ok(()),
                    _other => (),
                }
            }
        }
    }
}

fn update_tui(f: &mut Frame, powsup: &mut PowSup) {
    let mut message: Vec<Line> = Vec::new();
    let mut prt_err = |err: Error| {
        message.push(Line::from(Span::styled(
            err.to_string(),
            Style::default().fg(Color::Red),
        )))
    };

    let (max_v, max_i) = powsup.get_max().unwrap_or_else(|err| {
        prt_err(err);
        (Voltage(f64::NAN), Current(f64::NAN))
    });

    let display_out = powsup.get_out().unwrap_or_else(|err| {
        prt_err(err);
        "Error".to_string()
    });

    let (preset_v, preset_i) = powsup.get_preset().unwrap_or_else(|err| {
        prt_err(err);
        (Voltage(f64::NAN), Current(f64::NAN))
    });

    let (display_v, display_i, display_mode) = powsup.get_display().unwrap_or_else(|err| {
        prt_err(err);
        (Voltage(f64::NAN), Current(f64::NAN), String::from("--"))
    });

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
        .title(
            powsup
                .port
                .name()
                .map_or_else(|| " <unknown port> ".to_string(), |s| format!(" {s} ")),
        )
        .borders(Borders::ALL);
    let text = vec![
        Line::from("        Voltage   Current      "),
        Line::from(format!("Maximum: {max_v}   {max_i}      ")),
        Line::from(vec![
            Span::from(format!("Preset:  {preset_v}   {preset_i}  ")),
            Span::styled(
                format!("{display_out:5}"),
                if display_out == "On" {
                    Style::default().green().bold()
                } else {
                    Style::default().fg(Color::Red)
                },
            ),
        ]),
        Line::from(format!(
            "Actual:  {display_v}   {display_i}  {display_mode}  "
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
        Line::from("p => Power on     j => Zoom in (y-axis) "),
        Line::from("n => Power off    k => Zoom out (y-axis)"),
        Line::from("c => Power cycle                        "),
        Line::from("q => Quit                               "),
    ];
    let paragraph = Paragraph::new(text.clone())
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(paragraph, panes[1]);

    // middle block
    if powsup.y_max_offset + f64::from(preset_i) < 1.0 {
        powsup.y_max_offset = - f64::from(preset_i) + 1.0;
    }
    let y_max: f64 = f64::from(preset_i) + powsup.y_max_offset;
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
                    Span::raw(format!("{}", y_max * 0.25)),
                    Span::raw(format!("{}", y_max * 0.5)),
                    Span::raw(format!("{}", y_max * 0.75)),
                    Span::raw(format!("{}", y_max)),
                ])
                .bounds([0.0, y_max.into()]),
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

#[derive(Debug, Clone, Copy, From, Into, PartialEq)]
pub struct Current(f64);

impl fmt::Display for Current {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:5.2} A", self.0)
    }
}

#[derive(Debug, Clone, Copy, From, Into, PartialEq)]
pub struct Voltage(f64);

impl fmt::Display for Voltage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:5.2} V", self.0)
    }
}

pub struct PowSup {
    port: Box<dyn SerialPort>,
    cached_max: Option<(Voltage, Current)>,
    trend: CircularBuffer<300, (Voltage, Current)>,
    y_max_offset: f64,
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
            .with_context(|| format!("Failed to open the serial port \"{port}\""))?;
        port.clear(ClearBuffer::All)?;
        Ok(PowSup {
            port,
            cached_max: Option::None,
            trend: CircularBuffer::new(),
            y_max_offset: 0.0,
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

    pub fn get_display(&mut self) -> Result<(Voltage, Current, String)> {
        self.write("GETD\r")?;
        let reply = self.read()?;
        if reply.len() != 13 || &reply[10..] != "OK\r" {
            bail!(
                "Got an unexpected GETD reply from the power-supply: {:?}",
                &reply
            );
        }
        let v = format!("{}.{}", &reply[0..2], &reply[2..4])
            .parse::<f64>()
            .context("Failed to parse voltage from reply")?
            .into();
        let c = format!("{}.{}", &reply[4..6], &reply[6..8])
            .parse::<f64>()
            .context("Failed to parse current from reply")?
            .into();
        let cc = match &reply[8..9] {
            "0" => String::from("CV"),
            "1" => String::from("CC"),
            _other => bail!("Failed to parse const-current mode from reply"),
        };
        self.trend.push_back((v, c));
        Ok((v, c, cc))
    }

    pub fn get_preset(&mut self) -> Result<(Voltage, Current)> {
        self.write("GETS\r")?;
        let reply = self.read()?;
        if reply.len() != 10 || &reply[7..] != "OK\r" {
            bail!(
                "Got an unexpected GETS reply from the power-supply: {:?}",
                &reply
            );
        }
        let v = format!("{}.{}", &reply[0..2], &reply[2..3])
            .parse::<f64>()
            .context("Failed to parse voltage from reply")?
            .into();
        let c = format!("{}.{}", &reply[3..5], &reply[5..6])
            .parse::<f64>()
            .context("Failed to parse current from reply")?
            .into();
        Ok((v, c))
    }

    pub fn get_max(&mut self) -> Result<(Voltage, Current)> {
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
                .parse::<f64>()
                .context("Failed to parse voltage from reply")?
                .into();
            let c = format!("{}.{}", &reply[3..5], &reply[5..6])
                .parse::<f64>()
                .context("Failed to parse current from reply")?
                .into();
            self.cached_max = Some((v, c));
            Ok((v, c))
        }
    }

    pub fn get_out(&mut self) -> Result<String> {
        self.write("GOUT\r")?;
        let reply = self.read()?;
        if reply.len() != 5 || &reply[2..] != "OK\r" {
            bail!(
                "Got an unexpected GOUT reply from the power-supply: {:?}",
                &reply
            );
        }
        if &reply[..1] == "0" {
            Ok("On".to_string())
        } else if &reply[..1] == "1" {
            Ok("Off".to_string())
        } else {
            bail!("Got an unexpected value as GOUT reply: {:?}", &reply)
        }
    }

    pub fn status(&mut self, brief: bool) -> Result<()> {
        if !brief {
            let (v, i) = self.get_max()?;
            println!("Maximum: {v}  {i}");
            let (v, i) = self.get_preset()?;
            println!("Preset:  {v}  {i}");
        }
        let (v, i, cc) = self.get_display()?;
        println!("Display: {v}  {i}  {cc}");
        Ok(())
    }

    pub fn interactive(&mut self, args: &InteractiveArgs) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Do NOT unwrap the result here, only after we have put the
        // console back in a proper state.
        let result = run_app(&mut terminal, self, args);

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// List all available serial ports
    #[clap(short, long)]
    list_all: bool,
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
