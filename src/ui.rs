use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, Gauge, HighlightSpacing, Paragraph, Row,
        Sparkline, Table, Wrap,
    },
    Frame,
};

use crate::app::{
    ActiveTab, App, AppMode, InputMode, LogEntry, LogLevel, ProcessRow, ProcessSort,
};

const GIB: f64 = 1_073_741_824.0;
const MIB: f64 = 1_048_576.0;
const KIB: f64 = 1_024.0;
const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 16;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    let compact = area.height < 20;
    let top_height = if compact { 2 } else { 3 };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_height),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(area);

    render_top_bar(frame, outer[0], app, compact);

    let main_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
        .split(outer[1]);

    match app.active_tab {
        ActiveTab::Logs => render_log_table(frame, main_split[0], app),
        ActiveTab::Processes => render_processes_pane(frame, main_split[0], app),
    }

    render_sidebar(frame, main_split[1], app);
    render_bottom_bar(frame, outer[2], app);

    if app.popup_open {
        match app.active_tab {
            ActiveTab::Logs => {
                if let Some(entry) = app.selected_entry().cloned() {
                    render_log_popup(frame, area, app, &entry);
                } else {
                    app.popup_open = false;
                    app.popup_scroll = 0;
                }
            }
            ActiveTab::Processes => {
                if let Some(proc_row) = app.selected_process().cloned() {
                    render_process_popup(frame, area, app, &proc_row);
                } else {
                    app.popup_open = false;
                    app.popup_scroll = 0;
                }
            }
        }
    }
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let msg = format!(
        "Terminal too small ({}x{}).\nMinimum required: {}x{}.",
        area.width, area.height, MIN_WIDTH, MIN_HEIGHT
    );
    let block = rounded_block(" logpulse ").border_style(Style::default().fg(Color::Yellow));
    let paragraph = Paragraph::new(msg)
        .alignment(Alignment::Center)
        .block(block)
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(paragraph, area);
}

fn render_top_bar(frame: &mut Frame, area: Rect, app: &App, compact: bool) {
    let max_path_len = (area.width as usize).saturating_sub(56).max(8);
    let mode_text = match &app.mode {
        AppMode::Demo => " DEMO ".to_string(),
        AppMode::System => " SYSTEM ".to_string(),
        AppMode::File(path) => format!(
            " {} ",
            truncate_left(&path.display().to_string(), max_path_len)
        ),
    };
    let mode_style = match app.mode {
        AppMode::Demo => Style::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        AppMode::System => Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        AppMode::File(_) => Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    };

    let status_span = if app.paused {
        Span::styled(
            " PAUSED ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " ● LIVE ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    };

    let mut spans = vec![
        Span::styled(
            " logpulse ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        Span::styled(mode_text, mode_style),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
    ];

    match app.active_tab {
        ActiveTab::Logs => {
            spans.push(Span::styled("Total: ", Style::default().fg(Color::Gray)));
            spans.push(Span::styled(
                format_count(app.total_parsed),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!(" ERROR {} ", format_count(app.level_counts.error)),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        ActiveTab::Processes => {
            spans.push(Span::styled("Procs: ", Style::default().fg(Color::Gray)));
            spans.push(Span::styled(
                format_count(app.processes.len() as u64),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("Load {:.2}", app.host.load_one),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }

    spans.push(Span::raw("  "));
    spans.push(status_span);

    if area.width >= 76 {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(tab_pill("Logs", app.active_tab == ActiveTab::Logs));
        spans.push(Span::raw(" "));
        spans.push(tab_pill("Sys", app.active_tab == ActiveTab::Processes));
    }

    let block = if compact {
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
    };

    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

fn render_log_table(frame: &mut Frame, area: Rect, app: &mut App) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(3) as usize;
    app.table_page_size = inner_h.max(1);

    let msg_width = inner_w.saturating_sub(19);
    let selected_pos = app.table_state.selected();
    let rows: Vec<Row> = app
        .filtered_indices
        .iter()
        .enumerate()
        .filter_map(|(row_idx, &entry_idx)| {
            app.entries.get(entry_idx).map(|entry| (row_idx, entry))
        })
        .map(|(row_idx, entry)| {
            let is_selected = selected_pos == Some(row_idx);
            let clean_msg = sanitize_single_line(&entry.message);
            let truncated = truncate_ellipsis(&clean_msg, msg_width);
            let badge = if is_selected {
                selected_level_chip(entry.level)
            } else {
                level_chip(entry.level)
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    entry.timestamp.clone(),
                    Style::default().fg(Color::DarkGray),
                )),
                Cell::from(badge),
                Cell::from(Span::styled(
                    truncated,
                    Style::default().fg(message_color(entry.level)),
                )),
            ])
        })
        .collect();

    let count_title = if app.has_active_filter() {
        format!(
            " {}/{} matched ",
            format_count(app.filtered_indices.len() as u64),
            format_count(app.entries.len() as u64)
        )
    } else {
        format!(" {} buffered ", format_count(app.entries.len() as u64))
    };

    let block = rounded_block(" Logs ").title_top(
        Line::from(Span::styled(
            count_title,
            Style::default().fg(Color::Gray),
        ))
        .alignment(Alignment::Right),
    );

    let header = Row::new(vec![
        Cell::from("Time"),
        Cell::from("Level"),
        Cell::from("Message"),
    ])
    .style(
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .column_spacing(1)
    .highlight_symbol("▸ ")
    .highlight_spacing(HighlightSpacing::Always)
    .row_highlight_style(
        Style::default()
            .bg(Color::Indexed(237))
            .add_modifier(Modifier::BOLD),
    )
    .block(block);

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_processes_pane(frame: &mut Frame, area: Rect, app: &mut App) {
    let summary_h = if area.height >= 12 { 3 } else { 2 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(summary_h), Constraint::Min(4)])
        .split(area);

    render_host_summary(frame, chunks[0], app, summary_h == 2);

    let table_area = chunks[1];
    let inner_w = table_area.width.saturating_sub(2) as usize;
    let inner_h = table_area.height.saturating_sub(3) as usize;
    app.table_page_size = inner_h.max(1);

    // Fixed columns: highlight(2) + PID(7) + Status(8) + CPU(7) + MEM(9) + spacing(4) = 37
    let name_width = inner_w.saturating_sub(37).max(8);

    let rows: Vec<Row> = app
        .filtered_processes
        .iter()
        .filter_map(|&idx| app.processes.get(idx))
        .map(|proc_row| {
            let name = truncate_ellipsis(&sanitize_single_line(&proc_row.name), name_width);
            let status = truncate_ellipsis(&proc_row.status, 8);
            let cpu_text = format!("{:>6.1}%", proc_row.cpu_percent);
            let mem_text = format!("{:>8}", format_bytes_short(proc_row.memory_bytes));
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{:>6}", proc_row.pid),
                    Style::default().fg(Color::DarkGray),
                )),
                Cell::from(Span::styled(name, Style::default().fg(Color::White))),
                Cell::from(Span::styled(status, Style::default().fg(Color::Gray))),
                Cell::from(Span::styled(
                    cpu_text,
                    Style::default().fg(proc_cpu_color(proc_row.cpu_percent)),
                )),
                Cell::from(Span::styled(mem_text, Style::default().fg(Color::Cyan))),
            ])
        })
        .collect();

    let count_title = if app.has_active_filter() {
        format!(
            " {}/{} procs ",
            format_count(app.filtered_processes.len() as u64),
            format_count(app.processes.len() as u64)
        )
    } else {
        format!(" {} procs ", format_count(app.processes.len() as u64))
    };

    let block = rounded_block(" Processes ").title_top(
        Line::from(Span::styled(
            count_title,
            Style::default().fg(Color::Gray),
        ))
        .alignment(Alignment::Right),
    );

    let cpu_hdr = match app.process_sort {
        ProcessSort::Cpu => "CPU% ▼",
        ProcessSort::Memory => "CPU%",
    };
    let mem_hdr = match app.process_sort {
        ProcessSort::Memory => "MEM ▼",
        ProcessSort::Cpu => "MEM",
    };

    let header = Row::new(vec![
        Cell::from("   PID"),
        Cell::from("Name"),
        Cell::from("State"),
        Cell::from(format!("{cpu_hdr:>7}")),
        Cell::from(format!("{mem_hdr:>8}")),
    ])
    .style(
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .column_spacing(1)
    .highlight_symbol("▸ ")
    .highlight_spacing(HighlightSpacing::Always)
    .row_highlight_style(
        Style::default()
            .bg(Color::Indexed(237))
            .add_modifier(Modifier::BOLD),
    )
    .block(block);

    frame.render_stateful_widget(table, table_area, &mut app.process_table_state);
}

fn render_host_summary(frame: &mut Frame, area: Rect, app: &App, compact: bool) {
    let uptime = format_uptime(app.host.uptime_secs);
    let rx = format_rate(app.host.net_rx_bytes_per_sec);
    let tx = format_rate(app.host.net_tx_bytes_per_sec);
    let disk_used = app.host.disk_used_bytes as f64 / GIB;
    let disk_tot = app.host.disk_total_bytes as f64 / GIB;

    let hostname_str = truncate_ellipsis(&app.host.hostname, 18);
    let load_str = format!(
        "Load {:.2}/{:.2}/{:.2}",
        app.host.load_one, app.host.load_five, app.host.load_fifteen
    );

    let mut used_chars = 5 + hostname_str.chars().count() + 3 + 3 + uptime.chars().count() + 3 + load_str.chars().count();
    let mut spans = vec![
        Span::styled("Host ", Style::default().fg(Color::Gray)),
        Span::styled(
            hostname_str,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled("Up ", Style::default().fg(Color::Gray)),
        Span::styled(uptime, Style::default().fg(Color::Cyan)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(load_str, Style::default().fg(Color::White)),
    ];

    let inner_w = area.width.saturating_sub(2) as usize;
    let net_text = format!("↓{rx} ↑{tx}");
    let net_added = 3 + net_text.chars().count();
    if used_chars + net_added <= inner_w {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(net_text, Style::default().fg(Color::Green)));
        used_chars += net_added;
    }

    let disk_text = format!("Disk {disk_used:.0}/{disk_tot:.0}G");
    let disk_added = 3 + disk_text.chars().count();
    if used_chars + disk_added <= inner_w {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(disk_text, Style::default().fg(Color::Yellow)));
    }

    let block = if compact {
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray))
    } else {
        rounded_block(" Host ")
    };

    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

fn render_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    let compact = area.height < 15;
    if compact {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(area);

        render_compact_system_gauges(frame, chunks[0], app);
        render_sparkline(frame, chunks[1], app);
        match app.active_tab {
            ActiveTab::Logs => render_level_breakdown(frame, chunks[2], app, true),
            ActiveTab::Processes => render_host_io_breakdown(frame, chunks[2], app, true),
        }
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(4),
                Constraint::Length(6),
            ])
            .split(area);

        render_cpu_gauge(frame, chunks[0], app);
        render_ram_gauge(frame, chunks[1], app);
        render_sparkline(frame, chunks[2], app);
        match app.active_tab {
            ActiveTab::Logs => render_level_breakdown(frame, chunks[3], app, false),
            ActiveTab::Processes => render_host_io_breakdown(frame, chunks[3], app, false),
        }
    }
}

fn render_cpu_gauge(frame: &mut Frame, area: Rect, app: &App) {
    let ratio = (f64::from(app.cpu_percent) / 100.0).clamp(0.0, 1.0);
    let color = cpu_color(app.cpu_percent);
    let title = if area.width >= 16 {
        format!(" CPU ({}c) ", app.host.cpu_cores)
    } else {
        " CPU ".to_string()
    };
    let label = format!("{:.1}%", app.cpu_percent);

    let gauge = Gauge::default()
        .block(rounded_block(&title))
        .gauge_style(Style::default().fg(color).bg(Color::Indexed(236)))
        .use_unicode(true)
        .ratio(ratio)
        .label(Span::styled(
            label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(gauge, area);
}

fn render_ram_gauge(frame: &mut Frame, area: Rect, app: &App) {
    let (ratio, label) = ram_metrics(app, area.width);
    let color = ram_color(ratio);

    let gauge = Gauge::default()
        .block(rounded_block(" RAM "))
        .gauge_style(Style::default().fg(color).bg(Color::Indexed(236)))
        .use_unicode(true)
        .ratio(ratio)
        .label(Span::styled(
            label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(gauge, area);
}

fn render_compact_system_gauges(frame: &mut Frame, area: Rect, app: &App) {
    let block = rounded_block(" System ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    let cpu_ratio = (f64::from(app.cpu_percent) / 100.0).clamp(0.0, 1.0);
    let cpu_gauge = Gauge::default()
        .gauge_style(Style::default().fg(cpu_color(app.cpu_percent)).bg(Color::Indexed(236)))
        .use_unicode(true)
        .ratio(cpu_ratio)
        .label(Span::styled(
            format!("CPU {:.0}%", app.cpu_percent),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(cpu_gauge, rows[0]);

    let (ram_ratio, ram_label) = ram_metrics(app, area.width);
    let ram_gauge = Gauge::default()
        .gauge_style(Style::default().fg(ram_color(ram_ratio)).bg(Color::Indexed(236)))
        .use_unicode(true)
        .ratio(ram_ratio)
        .label(Span::styled(
            ram_label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(ram_gauge, rows[1]);
}

fn render_sparkline(frame: &mut Frame, area: Rect, app: &App) {
    let inner_w = area.width.saturating_sub(2) as usize;
    match app.active_tab {
        ActiveTab::Logs => {
            let title = if area.width >= 26 {
                format!(" Logs/s: {} (peak {}) ", app.current_rate, app.peak_rate)
            } else {
                format!(" {}/s pk:{} ", app.current_rate, app.peak_rate)
            };
            let skip = app.rate_history.len().saturating_sub(inner_w);
            let data: Vec<u64> = app.rate_history.iter().skip(skip).copied().collect();
            let sparkline = Sparkline::default()
                .block(rounded_block(&title))
                .data(data)
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(sparkline, area);
        }
        ActiveTab::Processes => {
            let title = if area.width >= 26 {
                format!(" CPU%: {:.0}% (peak {}%) ", app.cpu_percent, app.host.peak_cpu)
            } else {
                format!(" CPU {:.0}% pk:{} ", app.cpu_percent, app.host.peak_cpu)
            };
            let skip = app.host.cpu_history.len().saturating_sub(inner_w);
            let data: Vec<u64> = app.host.cpu_history.iter().skip(skip).copied().collect();
            let sparkline = Sparkline::default()
                .block(rounded_block(&title))
                .data(data)
                .max(100)
                .style(Style::default().fg(cpu_color(app.cpu_percent)));
            frame.render_widget(sparkline, area);
        }
    }
}

fn render_level_breakdown(frame: &mut Frame, area: Rect, app: &App, compact: bool) {
    let block = rounded_block(" Levels ");

    if compact {
        let line = Line::from(vec![
            Span::styled("E:", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{} ", app.level_counts.error)),
            Span::styled("W:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{} ", app.level_counts.warn)),
            Span::styled("I:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{} ", app.level_counts.info)),
            Span::styled("D:", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
            Span::raw(app.level_counts.debug.to_string()),
        ]);
        frame.render_widget(Paragraph::new(line).block(block), area);
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let items = [
        ("ERROR", Color::Red, app.level_counts.error),
        ("WARN", Color::Yellow, app.level_counts.warn),
        ("INFO", Color::Green, app.level_counts.info),
        ("DEBUG", Color::Blue, app.level_counts.debug),
    ];

    let lines: Vec<Line> = items
        .iter()
        .map(|&(name, color, count)| {
            let count_str = format_count(count);
            let used = 2 + name.len() + count_str.len();
            let gap = inner_w.saturating_sub(used).max(1);
            Line::from(vec![
                Span::styled("● ", Style::default().fg(color)),
                Span::styled(
                    name,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(gap)),
                Span::styled(
                    count_str,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_host_io_breakdown(frame: &mut Frame, area: Rect, app: &App, compact: bool) {
    let block = rounded_block(" Host I/O ");
    let rx = format_rate(app.host.net_rx_bytes_per_sec);
    let tx = format_rate(app.host.net_tx_bytes_per_sec);

    if compact {
        let line = Line::from(vec![
            Span::styled("↓", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{rx} ")),
            Span::styled("↑", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(tx),
        ]);
        frame.render_widget(Paragraph::new(line).block(block), area);
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let disk_str = format!(
        "{:.0}/{:.0}G",
        app.host.disk_used_bytes as f64 / GIB,
        app.host.disk_total_bytes as f64 / GIB
    );
    let swap_str = format!(
        "{:.1}/{:.1}G",
        app.host.swap_used_bytes as f64 / GIB,
        app.host.swap_total_bytes as f64 / GIB
    );

    let items = [
        ("Net ↓", Color::Green, rx),
        ("Net ↑", Color::Cyan, tx),
        ("Disk", Color::Yellow, disk_str),
        ("Swap", Color::Magenta, swap_str),
    ];

    let lines: Vec<Line> = items
        .iter()
        .map(|(label, color, val)| {
            let used = label.chars().count() + val.chars().count();
            let gap = inner_w.saturating_sub(used).max(1);
            Line::from(vec![
                Span::styled(
                    *label,
                    Style::default().fg(*color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(gap)),
                Span::styled(
                    val.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_bottom_bar(frame: &mut Frame, area: Rect, app: &App) {
    let is_search = app.input_mode == InputMode::Search;
    let border_color = if is_search {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);

    let mut line1_spans = vec![Span::styled(
        "/ ",
        Style::default()
            .fg(if is_search { Color::Cyan } else { Color::Gray })
            .add_modifier(Modifier::BOLD),
    )];

    if app.search_query.is_empty() && !is_search {
        let placeholder = match app.active_tab {
            ActiveTab::Logs => "search...",
            ActiveTab::Processes => "filter proc/pid...",
        };
        line1_spans.push(Span::styled(placeholder, Style::default().fg(Color::DarkGray)));
    } else {
        line1_spans.push(Span::styled(
            app.search_query.clone(),
            Style::default().fg(Color::White),
        ));
    }

    if app.has_active_filter() {
        let (matched, total) = match app.active_tab {
            ActiveTab::Logs => (app.filtered_indices.len(), app.entries.len()),
            ActiveTab::Processes => (app.filtered_processes.len(), app.processes.len()),
        };
        line1_spans.push(Span::styled(
            format!("  [{matched}/{total}]"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    line1_spans.push(Span::raw("   "));
    match app.active_tab {
        ActiveTab::Logs => {
            line1_spans.push(filter_chip("1:ERR", app.filters.error, Color::Red));
            line1_spans.push(Span::raw(" "));
            line1_spans.push(filter_chip("2:WARN", app.filters.warn, Color::Yellow));
            line1_spans.push(Span::raw(" "));
            line1_spans.push(filter_chip("3:INFO", app.filters.info, Color::Green));
            line1_spans.push(Span::raw(" "));
            line1_spans.push(filter_chip("4:DBG", app.filters.debug, Color::Blue));
        }
        ActiveTab::Processes => {
            line1_spans.push(filter_chip(
                "c:CPU",
                app.process_sort == ProcessSort::Cpu,
                Color::Cyan,
            ));
            line1_spans.push(Span::raw(" "));
            line1_spans.push(filter_chip(
                "m:MEM",
                app.process_sort == ProcessSort::Memory,
                Color::Magenta,
            ));
        }
    }

    let line2 = match app.active_tab {
        ActiveTab::Logs => {
            if inner.width >= 82 {
                Line::from(vec![
                    Span::styled("Tab", Style::default().fg(Color::Cyan)),
                    Span::styled(" sys  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("/", Style::default().fg(Color::Cyan)),
                    Span::styled(" search  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("1-4", Style::default().fg(Color::Cyan)),
                    Span::styled(" lvl  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("j/k", Style::default().fg(Color::Cyan)),
                    Span::styled(" nav  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Space", Style::default().fg(Color::Cyan)),
                    Span::styled(" pause  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Enter", Style::default().fg(Color::Cyan)),
                    Span::styled(" view  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("g/G", Style::default().fg(Color::Cyan)),
                    Span::styled(" top/live  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("q", Style::default().fg(Color::Cyan)),
                    Span::styled(" quit", Style::default().fg(Color::DarkGray)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("Tab", Style::default().fg(Color::Cyan)),
                    Span::styled(" sys ", Style::default().fg(Color::DarkGray)),
                    Span::styled("/", Style::default().fg(Color::Cyan)),
                    Span::styled(" find ", Style::default().fg(Color::DarkGray)),
                    Span::styled("1-4", Style::default().fg(Color::Cyan)),
                    Span::styled(" lvl ", Style::default().fg(Color::DarkGray)),
                    Span::styled("j/k", Style::default().fg(Color::Cyan)),
                    Span::styled(" ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Spc", Style::default().fg(Color::Cyan)),
                    Span::styled(" pause ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Ent", Style::default().fg(Color::Cyan)),
                    Span::styled(" view ", Style::default().fg(Color::DarkGray)),
                    Span::styled("q", Style::default().fg(Color::Cyan)),
                    Span::styled(" quit", Style::default().fg(Color::DarkGray)),
                ])
            }
        }
        ActiveTab::Processes => {
            if inner.width >= 82 {
                Line::from(vec![
                    Span::styled("Tab", Style::default().fg(Color::Cyan)),
                    Span::styled(" logs  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("/", Style::default().fg(Color::Cyan)),
                    Span::styled(" filter  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("c/m", Style::default().fg(Color::Cyan)),
                    Span::styled(" sort  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("j/k", Style::default().fg(Color::Cyan)),
                    Span::styled(" nav  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Space", Style::default().fg(Color::Cyan)),
                    Span::styled(" pause  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Enter", Style::default().fg(Color::Cyan)),
                    Span::styled(" info  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("g/G", Style::default().fg(Color::Cyan)),
                    Span::styled(" top/bot  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("q", Style::default().fg(Color::Cyan)),
                    Span::styled(" quit", Style::default().fg(Color::DarkGray)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("Tab", Style::default().fg(Color::Cyan)),
                    Span::styled(" logs ", Style::default().fg(Color::DarkGray)),
                    Span::styled("/", Style::default().fg(Color::Cyan)),
                    Span::styled(" find ", Style::default().fg(Color::DarkGray)),
                    Span::styled("c/m", Style::default().fg(Color::Cyan)),
                    Span::styled(" sort ", Style::default().fg(Color::DarkGray)),
                    Span::styled("j/k", Style::default().fg(Color::Cyan)),
                    Span::styled(" ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Spc", Style::default().fg(Color::Cyan)),
                    Span::styled(" pause ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Ent", Style::default().fg(Color::Cyan)),
                    Span::styled(" info ", Style::default().fg(Color::DarkGray)),
                    Span::styled("q", Style::default().fg(Color::Cyan)),
                    Span::styled(" quit", Style::default().fg(Color::DarkGray)),
                ])
            }
        }
    };

    let paragraph = Paragraph::new(vec![Line::from(line1_spans), line2]).block(block);
    frame.render_widget(paragraph, area);

    if is_search && inner.width > 2 && inner.height > 0 {
        let query_chars = u16::try_from(app.search_query.chars().count()).unwrap_or(u16::MAX);
        let cursor_x = inner
            .x
            .saturating_add(2)
            .saturating_add(query_chars)
            .min(inner.x + inner.width.saturating_sub(1));
        frame.set_cursor_position((cursor_x, inner.y));
    }
}

fn render_log_popup(frame: &mut Frame, area: Rect, app: &mut App, entry: &LogEntry) {
    let popup_area = centered_popup_rect(area);
    frame.render_widget(Clear, popup_area);

    let sanitized_raw = sanitize_single_line(&entry.raw);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("ID: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("#{}", entry.id),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   Time: ", Style::default().fg(Color::Gray)),
            Span::styled(entry.timestamp.clone(), Style::default().fg(Color::White)),
            Span::styled("   Level: ", Style::default().fg(Color::Gray)),
            level_chip(entry.level),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Raw Line:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            sanitized_raw.clone(),
            Style::default().fg(Color::White),
        )),
    ];

    let mut plain_lines = vec![
        format!(
            "ID: #{}   Time: {}   Level: {}",
            entry.id,
            entry.timestamp,
            entry.level.short_name()
        ),
        String::new(),
        "Raw Line:".to_string(),
        sanitized_raw,
    ];

    if let Some(ref val) = entry.json {
        if let Ok(pretty) = serde_json::to_string_pretty(val) {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Pretty JSON:",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            plain_lines.push(String::new());
            plain_lines.push("Pretty JSON:".to_string());

            for json_line in pretty.lines() {
                let clean = sanitize_single_line(json_line);
                lines.push(Line::from(Span::styled(
                    clean.clone(),
                    Style::default().fg(Color::Green),
                )));
                plain_lines.push(clean);
            }
        }
    }

    render_scrollable_popup(frame, popup_area, app, " Log Entry Details ", lines, &plain_lines);
}

fn render_process_popup(frame: &mut Frame, area: Rect, app: &mut App, proc_row: &ProcessRow) {
    let popup_area = centered_popup_rect(area);
    frame.render_widget(Clear, popup_area);

    let exe_display = if proc_row.exe.is_empty() {
        "<unavailable>".to_string()
    } else {
        sanitize_single_line(&proc_row.exe)
    };
    let name_clean = sanitize_single_line(&proc_row.name);
    let ram_str = format_bytes_short(proc_row.memory_bytes);
    let virt_str = format_bytes_short(proc_row.virtual_bytes);

    let lines = vec![
        Line::from(vec![
            Span::styled("PID: ", Style::default().fg(Color::Gray)),
            Span::styled(
                proc_row.pid.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   State: ", Style::default().fg(Color::Gray)),
            Span::styled(proc_row.status.clone(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Name: ", Style::default().fg(Color::Gray)),
            Span::styled(
                name_clean.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("CPU Usage: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{:.1}%", proc_row.cpu_percent),
                Style::default()
                    .fg(proc_cpu_color(proc_row.cpu_percent))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Physical RAM: ", Style::default().fg(Color::Gray)),
            Span::styled(ram_str.clone(), Style::default().fg(Color::Cyan)),
            Span::styled("   Virtual: ", Style::default().fg(Color::Gray)),
            Span::styled(virt_str.clone(), Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Executable Path:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            exe_display.clone(),
            Style::default().fg(Color::Green),
        )),
    ];

    let plain_lines = vec![
        format!("PID: {}   State: {}", proc_row.pid, proc_row.status),
        format!("Name: {name_clean}"),
        String::new(),
        format!("CPU Usage: {:.1}%", proc_row.cpu_percent),
        format!("Physical RAM: {ram_str}   Virtual: {virt_str}"),
        String::new(),
        "Executable Path:".to_string(),
        exe_display,
    ];

    render_scrollable_popup(frame, popup_area, app, " Process Details ", lines, &plain_lines);
}

fn centered_popup_rect(area: Rect) -> Rect {
    let popup_w = ((u32::from(area.width) * 82) / 100)
        .max(50)
        .min(u32::from(area.width.saturating_sub(2))) as u16;
    let popup_h = ((u32::from(area.height) * 78) / 100)
        .max(10)
        .min(u32::from(area.height.saturating_sub(2))) as u16;
    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    Rect::new(popup_x, popup_y, popup_w, popup_h)
}

fn render_scrollable_popup(
    frame: &mut Frame,
    popup_area: Rect,
    app: &mut App,
    title: &str,
    lines: Vec<Line<'_>>,
    plain_lines: &[String],
) {
    let inner_w = popup_area.width.saturating_sub(2) as usize;
    let inner_h = popup_area.height.saturating_sub(2) as usize;
    let total_rows: usize = plain_lines
        .iter()
        .map(|l| wrapped_rows_for_line(l, inner_w))
        .sum();
    let max_scroll = total_rows.saturating_sub(inner_h).min(u16::MAX as usize) as u16;
    app.popup_max_scroll = max_scroll;
    app.popup_scroll = app.popup_scroll.min(max_scroll);

    let block = rounded_block(title)
        .border_style(Style::default().fg(Color::Cyan))
        .title_bottom(
            Line::from(Span::styled(
                " j/k scroll • Esc/Enter close ",
                Style::default().fg(Color::Gray),
            ))
            .alignment(Alignment::Right),
        );

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.popup_scroll, 0));

    frame.render_widget(paragraph, popup_area);
}

fn rounded_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
}

fn tab_pill(label: &str, active: bool) -> Span<'static> {
    if active {
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(Color::Gray)
                .bg(Color::Indexed(236)),
        )
    }
}

fn level_chip(level: LogLevel) -> Span<'static> {
    let style = match level {
        LogLevel::Error => Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
        LogLevel::Warn => Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        LogLevel::Info => Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
        LogLevel::Debug => Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    };
    Span::styled(level.badge_label(), style)
}

fn selected_level_chip(level: LogLevel) -> Span<'static> {
    let fg = match level {
        LogLevel::Error => Color::LightRed,
        LogLevel::Warn => Color::Yellow,
        LogLevel::Info => Color::Green,
        LogLevel::Debug => Color::LightBlue,
    };
    Span::styled(
        level.badge_label(),
        Style::default().fg(fg).add_modifier(Modifier::BOLD),
    )
}

fn filter_chip(label: &str, enabled: bool, color: Color) -> Span<'static> {
    if enabled {
        let fg = match color {
            Color::Yellow | Color::Green | Color::Cyan => Color::Black,
            _ => Color::White,
        };
        Span::styled(
            format!(" {label} "),
            Style::default().fg(fg).bg(color).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(Color::DarkGray)
                .bg(Color::Indexed(235)),
        )
    }
}

fn message_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Error => Color::LightRed,
        LogLevel::Warn => Color::Yellow,
        LogLevel::Info => Color::White,
        LogLevel::Debug => Color::Gray,
    }
}

fn cpu_color(cpu: f32) -> Color {
    if cpu >= 85.0 {
        Color::Red
    } else if cpu >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn proc_cpu_color(cpu: f32) -> Color {
    if cpu >= 75.0 {
        Color::Red
    } else if cpu >= 20.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn ram_color(ratio: f64) -> Color {
    if ratio >= 0.85 {
        Color::Red
    } else if ratio >= 0.65 {
        Color::Yellow
    } else {
        Color::Cyan
    }
}

fn ram_metrics(app: &App, width: u16) -> (f64, String) {
    let used_gb = app.mem_used_bytes as f64 / GIB;
    let total_gb = app.mem_total_bytes as f64 / GIB;
    let ratio = if app.mem_total_bytes > 0 {
        (app.mem_used_bytes as f64 / app.mem_total_bytes as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let label = if width >= 24 {
        format!("{used_gb:.1} / {total_gb:.1} GB ({:.0}%)", ratio * 100.0)
    } else {
        format!("{used_gb:.1}/{total_gb:.1} GB")
    };
    (ratio, label)
}

fn format_bytes_short(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1}G", b / GIB)
    } else if b >= MIB {
        format!("{:.0}M", b / MIB)
    } else if b >= KIB {
        format!("{:.0}K", b / KIB)
    } else {
        format!("{bytes}B")
    }
}

fn format_rate(bytes_per_sec: u64) -> String {
    let b = bytes_per_sec as f64;
    if b >= MIB {
        format!("{:.1}M/s", b / MIB)
    } else if b >= KIB {
        format!("{:.0}K/s", b / KIB)
    } else {
        format!("{bytes_per_sec}B/s")
    }
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

fn sanitize_single_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else if max_chars == 1 {
        "…".to_string()
    } else {
        let mut out: String = s.chars().take(max_chars - 1).collect();
        out.push('…');
        out
    }
}

fn truncate_left(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else if max_chars == 1 {
        "…".to_string()
    } else {
        let skip = count - (max_chars - 1);
        let tail: String = s.chars().skip(skip).collect();
        format!("…{tail}")
    }
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (idx, ch) in s.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn wrapped_rows_for_line(s: &str, width: usize) -> usize {
    if width == 0 || s.is_empty() {
        return 1;
    }
    let mut rows = 1usize;
    let mut col = 0usize;
    for word in s.split_inclusive(char::is_whitespace) {
        let wlen = word.chars().count();
        if col > 0 && col + wlen > width {
            rows += 1;
            col = 0;
        }
        if wlen > width {
            rows += (wlen - 1) / width;
            col = wlen % width;
            if col == 0 {
                col = width;
            }
        } else {
            col += wlen;
        }
    }
    rows
}
