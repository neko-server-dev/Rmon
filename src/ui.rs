use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Gauge, Paragraph, Sparkline},
    Frame,
};

use crate::sys::common::{clamp_percent, human_bytes};

const BORDER: Color = Color::Cyan;
const TITLE: Color = Color::White;

#[derive(Clone)]
pub struct GaugeState {
    pub title: String,
    pub percent: u8,
    pub label: String,
    pub bar_color: Color,
}

impl GaugeState {
    pub fn new(title: impl Into<String>, bar_color: Color) -> Self {
        Self {
            title: title.into(),
            percent: 0,
            label: String::new(),
            bar_color,
        }
    }
}

pub struct Dashboard {
    pub cpu: Vec<GaugeState>,
    pub memory: GaugeState,
    pub swap: GaugeState,
    pub disks: Vec<GaugeState>,
    pub net_down_title: String,
    pub net_up_title: String,
    pub net_down_data: Vec<f64>,
    pub net_up_data: Vec<f64>,
}

pub fn color_for_usage(p: f64) -> Color {
    if p < 50.0 {
        Color::Green
    } else if p < 80.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

pub fn draw(frame: &mut Frame, area: Rect, dash: &Dashboard) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(45, 100),
            Constraint::Ratio(27, 100),
            Constraint::Ratio(25, 100),
            Constraint::Ratio(3, 100),
        ])
        .split(area);

    let cpu_max_cols = pick_max_cols(rows[0].width as usize, 40, 80, 120);
    render_gauge_section(frame, rows[0], &dash.cpu, "CPU", " (no CPU info)", cpu_max_cols);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(mid[0]);

    render_gauge(frame, left[0], &dash.memory);
    render_gauge(frame, left[1], &dash.swap);

    let disk_max_cols = pick_max_cols(mid[1].width as usize, 25, 50, 80);
    render_gauge_section(
        frame,
        mid[1],
        &dash.disks,
        "Disk",
        " (no disks detected)",
        disk_max_cols,
    );

    render_network(frame, rows[2], dash);
    render_help(frame, rows[3]);
}

pub fn update_cpu(cpu: &mut [GaugeState], percents: &[f64]) {
    for (i, p) in percents.iter().enumerate() {
        if i >= cpu.len() {
            break;
        }
        cpu[i].percent = clamp_percent(*p);
        cpu[i].label = format!("{p:.1}%");
        cpu[i].bar_color = color_for_usage(*p);
    }
}

pub fn update_memory(g: &mut GaugeState, pct: f64, used: u64, total: u64) {
    g.percent = clamp_percent(pct);
    g.title = format!("Memory {} / {}", human_bytes(used), human_bytes(total));
    g.label = format!("{pct:.1}%");
    g.bar_color = color_for_usage(pct);
}

pub fn update_swap(g: &mut GaugeState, data: Option<(f64, u64, u64)>) {
    match data {
        Some((pct, used, total)) => {
            g.percent = clamp_percent(pct);
            g.title = format!("Swap {} / {}", human_bytes(used), human_bytes(total));
            g.label = format!("{pct:.1}%");
            g.bar_color = color_for_usage(pct);
        }
        None => {
            g.title = "Swap".into();
            g.percent = 0;
            g.label = "n/a".into();
            g.bar_color = Color::Yellow;
        }
    }
}

pub fn update_disk(g: &mut GaugeState, name: &str, usage: Option<(f64, u64, u64)>) {
    let name = name.trim_end_matches(['\\', '/']);
    match usage {
        Some((pct, used, total)) => {
            g.percent = clamp_percent(pct);
            g.title = format!("Disk {name} {} / {}", human_bytes(used), human_bytes(total));
            g.label = format!("{pct:.1}%");
            g.bar_color = color_for_usage(pct);
        }
        None => {
            g.title = format!("Disk {name}");
            g.percent = 0;
            g.label = "n/a".into();
            g.bar_color = Color::Blue;
        }
    }
}

pub fn update_network_titles(
    down_title: &mut String,
    up_title: &mut String,
    recv_hist: &[f64],
    _sent_hist: &[f64],
    recv_rate: f64,
    sent_rate: f64,
    total_recv: u64,
    total_sent: u64,
) {
    if recv_hist.is_empty() {
        *down_title = " Down collecting...".into();
        *up_title = " Up collecting...".into();
        return;
    }
    *down_title = format!(
        " Down {}/s (total {})",
        human_bytes(recv_rate as u64),
        human_bytes(total_recv)
    );
    *up_title = format!(
        " Up {}/s (total {})",
        human_bytes(sent_rate as u64),
        human_bytes(total_sent)
    );
}

fn render_gauge(frame: &mut Frame, area: Rect, g: &GaugeState) {
    let gauge = Gauge::default()
        .block(gauge_block(&g.title))
        .gauge_style(Style::default().fg(g.bar_color))
        .label(g.label.clone())
        .percent(g.percent.into());
    frame.render_widget(gauge, area);
}

fn render_gauge_section(
    frame: &mut Frame,
    area: Rect,
    gauges: &[GaugeState],
    empty_title: &str,
    empty_msg: &str,
    max_cols: usize,
) {
    if gauges.is_empty() {
        let p = Paragraph::new(empty_msg)
            .block(empty_block(empty_title))
            .style(Style::default().fg(TITLE));
        frame.render_widget(p, area);
        return;
    }

    let (cols, per_col) = gauge_layout(area.height, gauges.len(), max_cols);
    let row_constraints = equal_row_constraints(area.height, per_col);
    let col_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, cols as u32); cols])
        .split(area);

    for c in 0..cols {
        let start = c * per_col;
        if start >= gauges.len() {
            break;
        }
        let end = (start + per_col).min(gauges.len());
        let row_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints.clone())
            .split(col_areas[c]);

        for slot in 0..per_col {
            if slot < end - start {
                render_gauge(frame, row_areas[slot], &gauges[start + slot]);
            } else {
                let pad = Paragraph::new("").block(Block::default().borders(Borders::NONE));
                frame.render_widget(pad, row_areas[slot]);
            }
        }
    }
}

fn equal_row_constraints(height: u16, per_col: usize) -> Vec<Constraint> {
    let per_col = per_col.max(1);
    let h = height.max(1) as usize;
    let base = h / per_col;
    let rem = h % per_col;
    (0..per_col)
        .map(|i| {
            let row_h = base + usize::from(i < rem);
            Constraint::Length(row_h.max(1) as u16)
        })
        .collect()
}

fn gauge_layout(height: u16, n: usize, max_cols: usize) -> (usize, usize) {
    const MIN_ROW: u16 = 3;
    let max_cols = max_cols.max(1);
    let height = height.max(MIN_ROW);

    let max_per_col = (height / MIN_ROW).max(1) as usize;
    let mut cols = ((n + max_per_col - 1) / max_per_col).max(1).min(max_cols);
    let mut per_col = (n + cols - 1) / cols;

    // 1行あたりが MIN_ROW 未満なら列を増やして行の高さを確保
    let mut row_h = height / per_col.max(1) as u16;
    while row_h < MIN_ROW && cols < n.min(max_cols) {
        cols += 1;
        per_col = (n + cols - 1) / cols;
        row_h = height / per_col.max(1) as u16;
    }

    (cols, per_col.max(1))
}

fn render_network(frame: &mut Frame, area: Rect, dash: &Dashboard) {
    let inner = Block::default()
        .title(" Network ")
        .title_style(Style::default().fg(TITLE))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER));

    let inner_area = inner.inner(area);
    frame.render_widget(inner, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(inner_area);

    let down_data = spark_data(&dash.net_down_data);
    let up_data = spark_data(&dash.net_up_data);

    let down = Sparkline::default()
        .block(
            Block::default()
                .title(dash.net_down_title.as_str())
                .title_style(Style::default().fg(Color::Green))
                .borders(Borders::NONE),
        )
        .style(Style::default().fg(Color::Green))
        .data(&down_data);

    let up = Sparkline::default()
        .block(
            Block::default()
                .title(dash.net_up_title.as_str())
                .title_style(Style::default().fg(Color::Magenta))
                .borders(Borders::NONE),
        )
        .style(Style::default().fg(Color::Magenta))
        .data(&up_data);

    frame.render_widget(down, rows[0]);
    frame.render_widget(up, rows[1]);
}

fn spark_data(values: &[f64]) -> Vec<u64> {
    if values.is_empty() {
        return vec![];
    }
    let max = values.iter().copied().fold(0.0f64, f64::max);
    let max = if max <= 0.0 { 1.0 } else { max };
    values
        .iter()
        .map(|v| ((v / max) * 100.0).round() as u64)
        .collect()
}

fn render_help(frame: &mut Frame, area: Rect) {
    let help = Paragraph::new(" [q] quit  [Ctrl+C] quit ")
        .style(Style::default().fg(TITLE));
    frame.render_widget(help, area);
}

fn gauge_block(title: &str) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .title_style(Style::default().fg(TITLE))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
}

fn empty_block(title: &str) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .title_style(Style::default().fg(TITLE))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
}

fn pick_max_cols(width: usize, t1: usize, t2: usize, t3: usize) -> usize {
    if width < t1 {
        1
    } else if width < t2 {
        2
    } else if width < t3 {
        3
    } else {
        4
    }
}
