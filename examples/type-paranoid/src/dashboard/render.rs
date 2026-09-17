use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, BorderType, Chart, Dataset, Gauge, GraphType, Paragraph, Row, Table},
};

use super::state::{State, bytes};

const BACKGROUND: Color = Color::Rgb(12, 17, 25);
const PANEL: Color = Color::Rgb(19, 27, 39);
const BORDER: Color = Color::Rgb(45, 61, 78);
const TEXT: Color = Color::Rgb(219, 230, 240);
const MUTED: Color = Color::Rgb(130, 150, 171);
const OUT: Color = Color::Rgb(255, 176, 95);
const IN: Color = Color::Rgb(70, 211, 208);
const GREEN: Color = Color::Rgb(143, 217, 149);

fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL).fg(TEXT))
}

pub fn draw(frame: &mut Frame, state: &State) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(Style::default().bg(BACKGROUND).fg(TEXT)),
        area,
    );
    if area.width < 72 || area.height < 24 {
        frame.render_widget(Paragraph::new(format!(
            "TypeParanoid | {}\n\nOUT {}/s   IN {}/s\n\n{} candidates\n\nEnlarge terminal to 72 x 24 for graphs.\nq / Esc / Ctrl-C to quit",
            state.interface, bytes(state.outbound_rate), bytes(state.inbound_rate), state.candidates
        )).block(panel(" Live capture ")), area);
        return;
    }
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(10),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(2),
    ])
    .split(area);
    header(frame, state, layout[0]);
    stats(frame, state, layout[1]);
    graphs(frame, state, layout[2]);
    let progress =
        ((state.elapsed - state.last_window_at) / state.window_secs as f64).clamp(0.0, 1.0);
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(IN).bg(PANEL))
            .ratio(progress)
            .label(format!(
                " window {} / {}s   ·   {} peers in last window ",
                state.window + 1,
                state.window_secs,
                state.peers
            )),
        layout[3],
    );
    alerts(frame, state, layout[4]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(state.notice.as_str()).fg(MUTED),
            Line::from(vec![
                Span::styled(" q / Esc / Ctrl-C ", Style::default().fg(IN).bold()),
                Span::raw("quit · Payload bytes · PID unavailable · Kernel drops unknown"),
            ])
            .fg(MUTED),
        ]),
        layout[5],
    );
}

fn header(frame: &mut Frame, state: &State, area: Rect) {
    let minutes = state.elapsed as u64 / 60;
    let seconds = state.elapsed as u64 % 60;
    let label = format!(
        "  {}   ·   {:02}:{:02}   ·   {}",
        state.interface,
        minutes,
        seconds,
        if state.assess {
            "TypeSafe enabled"
        } else {
            "Local analysis"
        }
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" TYPE", Style::default().fg(TEXT).bold()),
                Span::styled("PARANOID", Style::default().fg(IN).bold()),
                Span::styled("    ● LIVE", Style::default().fg(GREEN)),
                Span::styled(label, Style::default().fg(MUTED)),
            ]),
            Line::from(" Trust types. Question traffic.").fg(MUTED),
        ]),
        area,
    );
}

fn stats(frame: &mut Frame, state: &State, area: Rect) {
    let cells = Layout::horizontal([Constraint::Percentage(25); 4]).split(area);
    for (index, title, value, detail, color) in [
        (
            0,
            " OUTBOUND ",
            format!("{}/s", bytes(state.outbound_rate)),
            format!("{} total", bytes(state.sent as f64)),
            OUT,
        ),
        (
            1,
            " INBOUND ",
            format!("{}/s", bytes(state.inbound_rate)),
            format!("{} total", bytes(state.received as f64)),
            IN,
        ),
        (
            2,
            " REVIEW ",
            format!("{} candidates", state.candidates),
            format!("{} in flight", state.inflight),
            OUT,
        ),
        (
            3,
            " CAPTURE ",
            format!("{} queue drops", state.dropped),
            format!("{} parse / {} flow", state.unparsed, state.untracked),
            if state.dropped > 0 { OUT } else { GREEN },
        ),
    ] {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(value).fg(color).bold(),
                Line::from(detail).fg(MUTED),
            ])
            .alignment(Alignment::Center)
            .block(panel(title)),
            cells[index],
        );
    }
}

fn graphs(frame: &mut Frame, state: &State, area: Rect) {
    let areas = Layout::horizontal([Constraint::Percentage(50); 2]).split(area);
    // A shared vertical scale makes outgoing and incoming volume directly comparable.
    let maximum = state
        .outbound
        .iter()
        .chain(state.inbound.iter())
        .map(|p| p.1)
        .fold(1024.0_f64, f64::max)
        * 1.15;
    for (rect, history, title, color) in [
        (areas[0], &state.outbound, " OUTBOUND / last 120s ", OUT),
        (areas[1], &state.inbound, " INBOUND / last 120s ", IN),
    ] {
        let data: Vec<_> = history
            .iter()
            .map(|&(time, rate)| (time - state.elapsed, rate))
            .collect();
        let dataset = Dataset::default()
            .data(&data)
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(color));
        frame.render_widget(
            Chart::new(vec![dataset])
                .block(panel(title))
                .x_axis(
                    Axis::default()
                        .bounds([-120.0, 0.0])
                        .style(Style::default().fg(BORDER))
                        .labels(["-120s", "-60s", "now"].map(|text| Line::from(text).fg(MUTED))),
                )
                .y_axis(
                    Axis::default()
                        .bounds([0.0, maximum])
                        .style(Style::default().fg(BORDER))
                        .labels(vec![
                            Line::from("0").fg(MUTED),
                            Line::from(format!("{}/s", bytes(maximum))).fg(MUTED),
                        ]),
                ),
            rect,
        );
    }
}

fn alerts(frame: &mut Frame, state: &State, area: Rect) {
    let block = panel(" RECENT CANDIDATES / latest 32 ");
    if state.alerts.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No candidates in completed windows. Collecting traffic and local history.",
            )
            .style(Style::default().fg(MUTED))
            .block(block),
            area,
        );
        return;
    }
    let wide = area.width >= 110;
    let rows = state.alerts.iter().enumerate().map(|(index, alert)| {
        let mut cells = vec![
            alert.window.to_string(),
            format!("{} {:?}", alert.peer.remote, alert.peer.protocol),
            bytes(alert.sent as f64),
        ];
        if wide {
            cells.push(alert.signals.clone());
        }
        cells.push(alert.assessment.clone());
        Row::new(cells).style(Style::default().fg(TEXT).bg(if index % 2 == 0 {
            PANEL
        } else {
            BACKGROUND
        }))
    });
    let mut headings = vec!["Win", "Destination / protocol", "Outbound"];
    let mut widths = vec![
        Constraint::Length(4),
        Constraint::Min(25),
        Constraint::Length(12),
    ];
    if wide {
        headings.push("Local signals");
        widths.push(Constraint::Percentage(28));
    }
    headings.push("TypeSafe");
    widths.push(Constraint::Length(25));
    frame.render_widget(
        Table::new(rows, widths)
            .header(Row::new(headings).style(Style::default().fg(IN).bold()))
            .column_spacing(1)
            .block(block),
        area,
    );
}
