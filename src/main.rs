mod sys;
mod ui;

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use sys::common::UPDATE_INTERVAL_MS;
use sys::Monitor;
use ui::{
    update_cpu, update_disk, update_memory, update_network_titles, update_swap, Dashboard,
    GaugeState,
};

fn main() {
    if let Err(e) = run() {
        restore_terminal();
        eprintln!("rmon: {e}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    terminal.show_cursor()?;

    result
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = execute!(stdout, LeaveAlternateScreen, crossterm::cursor::Show);
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut mon = Monitor::new();

    let cpu_percents = mon.cpu_percents();
    let mut cpu: Vec<GaugeState> = cpu_percents
        .iter()
        .enumerate()
        .map(|(i, _)| GaugeState::new(format!("CPU {i}"), ratatui::style::Color::Green))
        .collect();
    if cpu.is_empty() {
        cpu.push(GaugeState::new("CPU 0", ratatui::style::Color::Green));
    }

    let mounts = mon.mountpoints();
    let disks: Vec<GaugeState> = mounts
        .iter()
        .map(|m| GaugeState::new(format!("Disk {m}"), ratatui::style::Color::Blue))
        .collect();

    let mut dash = Dashboard {
        cpu,
        memory: GaugeState::new("Memory", ratatui::style::Color::Magenta),
        swap: GaugeState::new("Swap", ratatui::style::Color::Yellow),
        disks,
        net_down_title: " Down collecting...".into(),
        net_up_title: " Up collecting...".into(),
        net_down_data: Vec::new(),
        net_up_data: Vec::new(),
    };

    update_metrics(&mut mon, &mut dash, &mounts);

    loop {
        terminal.draw(|f| ui::draw(f, f.area(), &dash))?;

        if event::poll(Duration::from_millis(UPDATE_INTERVAL_MS))? {
            while event::poll(Duration::from_millis(0))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            return Ok(())
                        }
                        _ => {}
                    },
                    Event::Resize(_, _) => {
                        terminal.clear()?;
                    }
                    _ => {}
                }
            }
        }

        update_metrics(&mut mon, &mut dash, &mounts);
    }
}

fn update_metrics(mon: &mut Monitor, dash: &mut Dashboard, mounts: &[String]) {
    update_cpu(&mut dash.cpu, &mon.cpu_percents());
    if let Some((pct, used, total)) = mon.memory() {
        update_memory(&mut dash.memory, pct, used, total);
    }
    update_swap(&mut dash.swap, mon.swap());
    for (i, m) in mounts.iter().enumerate() {
        if i >= dash.disks.len() {
            break;
        }
        update_disk(&mut dash.disks[i], m, mon.disk_usage(m));
    }
    mon.sample_network();
    let (rd, ru, rr, sr, tr, ts) = mon.network();
    update_network_titles(
        &mut dash.net_down_title,
        &mut dash.net_up_title,
        rd,
        ru,
        rr,
        sr,
        tr,
        ts,
    );
    dash.net_down_data = rd.to_vec();
    dash.net_up_data = ru.to_vec();
}
