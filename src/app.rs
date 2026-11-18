use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, SecondsFormat};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rand::Rng;
use ratatui::widgets::TableState;
use serde::Deserialize;
use serde_json::{json, Value};
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate,
    RefreshKind, System,
};

pub const MAX_ENTRIES: usize = 10_000;
pub const SPARKLINE_WINDOW: usize = 60;
const TAIL_INIT_BYTES: u64 = 256 * 1024;
const TAIL_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn badge_label(self) -> &'static str {
        match self {
            LogLevel::Error => " ERROR ",
            LogLevel::Warn => " WARN ",
            LogLevel::Info => " INFO ",
            LogLevel::Debug => " DEBUG ",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub id: u64,
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
    pub raw: String,
    pub json: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ProcessRow {
    pub pid: u32,
    pub name: String,
    pub status: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub virtual_bytes: u64,
    pub exe: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Logs,
    Processes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSort {
    Cpu,
    Memory,
}

#[derive(Debug, Clone)]
pub struct HostMetrics {
    pub hostname: String,
    pub uptime_secs: u64,
    pub cpu_cores: usize,
    pub load_one: f64,
    pub load_five: f64,
    pub load_fifteen: f64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub net_rx_bytes_per_sec: u64,
    pub net_tx_bytes_per_sec: u64,
    pub cpu_history: VecDeque<u64>,
    pub peak_cpu: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
}

#[derive(Debug, Clone)]
pub enum AppMode {
    Demo,
    System,
    File(PathBuf),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LevelCounts {
    pub debug: u64,
    pub info: u64,
    pub warn: u64,
    pub error: u64,
}

impl LevelCounts {
    pub fn increment(&mut self, level: LogLevel) {
        match level {
            LogLevel::Debug => self.debug += 1,
            LogLevel::Info => self.info += 1,
            LogLevel::Warn => self.warn += 1,
            LogLevel::Error => self.error += 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LevelFilters {
    pub error: bool,
    pub warn: bool,
    pub info: bool,
    pub debug: bool,
}

impl Default for LevelFilters {
    fn default() -> Self {
        Self {
            error: true,
            warn: true,
            info: true,
            debug: true,
        }
    }
}

impl LevelFilters {
    pub fn is_enabled(self, level: LogLevel) -> bool {
        match level {
            LogLevel::Error => self.error,
            LogLevel::Warn => self.warn,
            LogLevel::Info => self.info,
            LogLevel::Debug => self.debug,
        }
    }

    pub fn all_enabled(self) -> bool {
        self.error && self.warn && self.info && self.debug
    }

    pub fn toggle(&mut self, level: LogLevel) {
        match level {
            LogLevel::Error => self.error = !self.error,
            LogLevel::Warn => self.warn = !self.warn,
            LogLevel::Info => self.info = !self.info,
            LogLevel::Debug => self.debug = !self.debug,
        }
    }
}

pub struct App {
    pub mode: AppMode,
    pub active_tab: ActiveTab,
    pub entries: VecDeque<LogEntry>,
    pub filtered_indices: Vec<usize>,
    pub total_parsed: u64,
    pub level_counts: LevelCounts,
    pub table_state: TableState,
    pub selected_id: Option<u64>,
    pub paused: bool,
    pub input_mode: InputMode,
    pub search_query: String,
    pub filters: LevelFilters,
    pub popup_open: bool,
    pub popup_scroll: u16,
    pub popup_max_scroll: u16,
    pub table_page_size: usize,
    pub should_quit: bool,
    pub cpu_percent: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub rate_history: VecDeque<u64>,
    pub current_rate: u64,
    pub peak_rate: u64,
    pub processes: Vec<ProcessRow>,
    pub filtered_processes: Vec<usize>,
    pub process_table_state: TableState,
    pub selected_pid: Option<u32>,
    pub process_sort: ProcessSort,
    pub host: HostMetrics,
    logs_this_second: u64,
    last_net_sample: Instant,
    sys: System,
    networks: Networks,
    disks: Disks,
}

impl App {
    pub fn new(mode: AppMode) -> Result<Self> {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_usage()),
        )
        .map_err(|e| anyhow!("failed to initialize system monitor: {e}"))?;

        sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        let networks = Networks::new_with_refreshed_list()
            .map_err(|e| anyhow!("failed to initialize network metrics: {e}"))?;
        let disks = Disks::new_with_refreshed_list()
            .map_err(|e| anyhow!("failed to initialize disk metrics: {e}"))?;
        let (disk_used_bytes, disk_total_bytes) = read_primary_disk(&disks);
        let load = System::load_average().unwrap_or(sysinfo::LoadAvg {
            one: 0.0,
            five: 0.0,
            fifteen: 0.0,
        });

        let active_tab = match mode {
            AppMode::System => ActiveTab::Processes,
            AppMode::Demo | AppMode::File(_) => ActiveTab::Logs,
        };

        let host = HostMetrics {
            hostname: System::host_name().unwrap_or_else(|_| "localhost".to_string()),
            uptime_secs: System::uptime().unwrap_or(0),
            cpu_cores: sys.cpus().len().max(1),
            load_one: load.one,
            load_five: load.five,
            load_fifteen: load.fifteen,
            swap_used_bytes: sys.used_swap(),
            swap_total_bytes: sys.total_swap(),
            disk_used_bytes,
            disk_total_bytes,
            net_rx_bytes_per_sec: 0,
            net_tx_bytes_per_sec: 0,
            cpu_history: VecDeque::from(vec![0; SPARKLINE_WINDOW]),
            peak_cpu: 0,
        };

        let mut app = Self {
            mode,
            active_tab,
            entries: VecDeque::with_capacity(MAX_ENTRIES),
            filtered_indices: Vec::new(),
            total_parsed: 0,
            level_counts: LevelCounts::default(),
            table_state: TableState::default(),
            selected_id: None,
            paused: false,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            filters: LevelFilters::default(),
            popup_open: false,
            popup_scroll: 0,
            popup_max_scroll: 0,
            table_page_size: 20,
            should_quit: false,
            cpu_percent: 0.0,
            mem_used_bytes: sys.used_memory(),
            mem_total_bytes: sys.total_memory(),
            rate_history: VecDeque::from(vec![0; SPARKLINE_WINDOW]),
            current_rate: 0,
            peak_rate: 0,
            processes: Vec::new(),
            filtered_processes: Vec::new(),
            process_table_state: TableState::default(),
            selected_pid: None,
            process_sort: ProcessSort::Cpu,
            host,
            logs_this_second: 0,
            last_net_sample: Instant::now(),
            sys,
            networks,
            disks,
        };

        app.rebuild_processes();
        Ok(app)
    }

    pub fn drain_incoming(&mut self, rx: &Receiver<LogEntry>) {
        let query_lower = self.search_query.to_ascii_lowercase();
        let mut evicted_matching = 0usize;
        let mut received_any = false;

        while let Ok(entry) = rx.try_recv() {
            received_any = true;
            if self.entries.len() == MAX_ENTRIES {
                if let Some(removed) = self.entries.pop_front() {
                    if Self::entry_matches(&removed, self.filters, &query_lower) {
                        evicted_matching += 1;
                    }
                }
            }
            self.total_parsed += 1;
            self.logs_this_second += 1;
            self.level_counts.increment(entry.level);
            self.entries.push_back(entry);
        }

        if received_any {
            self.refresh_after_ingest(evicted_matching);
        }
    }

    pub fn on_tick(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.cpu_percent = self.sys.global_cpu_usage().clamp(0.0, 100.0);
        self.mem_used_bytes = self.sys.used_memory();
        self.mem_total_bytes = self.sys.total_memory();
        self.host.cpu_cores = self.sys.cpus().len().max(1);
        self.host.uptime_secs = System::uptime().unwrap_or(self.host.uptime_secs);
        if let Ok(load) = System::load_average() {
            self.host.load_one = load.one;
            self.host.load_five = load.five;
            self.host.load_fifteen = load.fifteen;
        }
        self.host.swap_used_bytes = self.sys.used_swap();
        self.host.swap_total_bytes = self.sys.total_swap();

        let elapsed = self.last_net_sample.elapsed().as_secs_f64().max(0.1);
        self.last_net_sample = Instant::now();
        self.networks.refresh(true);
        let (mut rx_bytes, mut tx_bytes) = (0u64, 0u64);
        for (iface, data) in &self.networks {
            if iface.to_string_lossy().starts_with("lo") {
                continue;
            }
            rx_bytes = rx_bytes.saturating_add(data.received());
            tx_bytes = tx_bytes.saturating_add(data.transmitted());
        }
        self.host.net_rx_bytes_per_sec = (rx_bytes as f64 / elapsed).round() as u64;
        self.host.net_tx_bytes_per_sec = (tx_bytes as f64 / elapsed).round() as u64;

        self.disks.refresh(true);
        let (disk_used, disk_total) = read_primary_disk(&self.disks);
        self.host.disk_used_bytes = disk_used;
        self.host.disk_total_bytes = disk_total;

        if self.host.cpu_history.len() == SPARKLINE_WINDOW {
            self.host.cpu_history.pop_front();
        }
        let cpu_point = self.cpu_percent.round().clamp(0.0, 100.0) as u64;
        self.host.cpu_history.push_back(cpu_point);
        self.host.peak_cpu = self.host.cpu_history.iter().copied().max().unwrap_or(0);

        self.current_rate = self.logs_this_second;
        self.logs_this_second = 0;
        if self.rate_history.len() == SPARKLINE_WINDOW {
            self.rate_history.pop_front();
        }
        self.rate_history.push_back(self.current_rate);
        self.peak_rate = self.rate_history.iter().copied().max().unwrap_or(0);

        if !self.paused {
            self.sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
            self.rebuild_processes();
        }
    }

    pub fn has_active_filter(&self) -> bool {
        match self.active_tab {
            ActiveTab::Logs => !self.search_query.is_empty() || !self.filters.all_enabled(),
            ActiveTab::Processes => !self.search_query.is_empty(),
        }
    }

    pub fn selected_entry(&self) -> Option<&LogEntry> {
        let pos = self.table_state.selected()?;
        let idx = *self.filtered_indices.get(pos)?;
        self.entries.get(idx)
    }

    pub fn selected_process(&self) -> Option<&ProcessRow> {
        let pos = self.process_table_state.selected()?;
        let idx = *self.filtered_processes.get(pos)?;
        self.processes.get(idx)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if self.popup_open {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.popup_open = false;
                    self.popup_scroll = 0;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.popup_scroll = self.popup_scroll.saturating_add(1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.popup_scroll = self.popup_scroll.saturating_sub(1);
                }
                KeyCode::PageDown => {
                    self.popup_scroll = self.popup_scroll.saturating_add(10);
                }
                KeyCode::PageUp => {
                    self.popup_scroll = self.popup_scroll.saturating_sub(10);
                }
                KeyCode::Char('g') => {
                    self.popup_scroll = 0;
                }
                KeyCode::Char('G') => {
                    self.popup_scroll = self.popup_max_scroll;
                }
                KeyCode::Char('q') => {
                    self.should_quit = true;
                }
                _ => {}
            }
            return;
        }

        if self.input_mode == InputMode::Search {
            match key.code {
                KeyCode::Esc => {
                    self.search_query.clear();
                    self.input_mode = InputMode::Normal;
                    self.refresh_active_filter();
                }
                KeyCode::Enter => {
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.refresh_active_filter();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.search_query.push(c);
                    self.refresh_active_filter();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => self.toggle_tab(),
            KeyCode::Char('/') => self.input_mode = InputMode::Search,
            KeyCode::Char(' ') => self.toggle_pause(),
            KeyCode::Char('1') if self.active_tab == ActiveTab::Logs => {
                self.toggle_level(LogLevel::Error);
            }
            KeyCode::Char('2') if self.active_tab == ActiveTab::Logs => {
                self.toggle_level(LogLevel::Warn);
            }
            KeyCode::Char('3') if self.active_tab == ActiveTab::Logs => {
                self.toggle_level(LogLevel::Info);
            }
            KeyCode::Char('4') if self.active_tab == ActiveTab::Logs => {
                self.toggle_level(LogLevel::Debug);
            }
            KeyCode::Char('c') if self.active_tab == ActiveTab::Processes => {
                self.set_process_sort(ProcessSort::Cpu);
            }
            KeyCode::Char('m') if self.active_tab == ActiveTab::Processes => {
                self.set_process_sort(ProcessSort::Memory);
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_down(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(1),
            KeyCode::PageDown => self.move_down(self.table_page_size.max(1)),
            KeyCode::PageUp => self.move_up(self.table_page_size.max(1)),
            KeyCode::Char('g') => self.jump_top(),
            KeyCode::Char('G') => self.jump_bottom_resume(),
            KeyCode::Enter if self.can_open_popup() => {
                self.popup_open = true;
                self.popup_scroll = 0;
            }
            _ => {}
        }
    }

    fn toggle_tab(&mut self) {
        self.popup_open = false;
        self.popup_scroll = 0;
        self.active_tab = match self.active_tab {
            ActiveTab::Logs => ActiveTab::Processes,
            ActiveTab::Processes => ActiveTab::Logs,
        };
        self.refresh_active_filter();
    }

    fn can_open_popup(&self) -> bool {
        match self.active_tab {
            ActiveTab::Logs => self.selected_entry().is_some(),
            ActiveTab::Processes => self.selected_process().is_some(),
        }
    }

    fn set_process_sort(&mut self, sort: ProcessSort) {
        if self.process_sort != sort {
            self.process_sort = sort;
            self.sort_and_filter_processes();
        }
    }

    fn toggle_level(&mut self, level: LogLevel) {
        self.filters.toggle(level);
        self.refresh_after_filter_change();
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            match self.active_tab {
                ActiveTab::Logs if !self.filtered_indices.is_empty() => {
                    let last = self.filtered_indices.len() - 1;
                    self.selected_id = Some(self.entries[self.filtered_indices[last]].id);
                    self.table_state.select(Some(last));
                }
                ActiveTab::Processes => {
                    self.sys.refresh_processes_specifics(
                        ProcessesToUpdate::All,
                        true,
                        ProcessRefreshKind::nothing().with_cpu().with_memory(),
                    );
                    self.rebuild_processes();
                }
                _ => {}
            }
        }
    }

    fn move_up(&mut self, step: usize) {
        match self.active_tab {
            ActiveTab::Logs => self.move_selection_up(step),
            ActiveTab::Processes => self.move_process_up(step),
        }
    }

    fn move_down(&mut self, step: usize) {
        match self.active_tab {
            ActiveTab::Logs => self.move_selection_down(step),
            ActiveTab::Processes => self.move_process_down(step),
        }
    }

    fn move_selection_up(&mut self, step: usize) {
        self.paused = true;
        if self.filtered_indices.is_empty() {
            return;
        }
        let current = self.table_state.selected().unwrap_or(0);
        let next = current.saturating_sub(step);
        self.table_state.select(Some(next));
        self.selected_id = Some(self.entries[self.filtered_indices[next]].id);
    }

    fn move_selection_down(&mut self, step: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let last = self.filtered_indices.len() - 1;
        let current = self.table_state.selected().unwrap_or(0);
        let next = current.saturating_add(step).min(last);
        self.table_state.select(Some(next));
        self.selected_id = Some(self.entries[self.filtered_indices[next]].id);
    }

    fn move_process_up(&mut self, step: usize) {
        if self.filtered_processes.is_empty() {
            return;
        }
        let current = self.process_table_state.selected().unwrap_or(0);
        let next = current.saturating_sub(step);
        self.process_table_state.select(Some(next));
        self.selected_pid = Some(self.processes[self.filtered_processes[next]].pid);
    }

    fn move_process_down(&mut self, step: usize) {
        if self.filtered_processes.is_empty() {
            return;
        }
        let last = self.filtered_processes.len() - 1;
        let current = self.process_table_state.selected().unwrap_or(0);
        let next = current.saturating_add(step).min(last);
        self.process_table_state.select(Some(next));
        self.selected_pid = Some(self.processes[self.filtered_processes[next]].pid);
    }

    fn jump_top(&mut self) {
        match self.active_tab {
            ActiveTab::Logs => {
                self.paused = true;
                if self.filtered_indices.is_empty() {
                    return;
                }
                self.table_state.select(Some(0));
                self.selected_id = Some(self.entries[self.filtered_indices[0]].id);
            }
            ActiveTab::Processes => {
                if self.filtered_processes.is_empty() {
                    return;
                }
                self.process_table_state.select(Some(0));
                self.selected_pid = Some(self.processes[self.filtered_processes[0]].pid);
            }
        }
    }

    fn jump_bottom_resume(&mut self) {
        match self.active_tab {
            ActiveTab::Logs => {
                self.paused = false;
                if self.filtered_indices.is_empty() {
                    return;
                }
                let last = self.filtered_indices.len() - 1;
                self.table_state.select(Some(last));
                self.selected_id = Some(self.entries[self.filtered_indices[last]].id);
            }
            ActiveTab::Processes => {
                if self.filtered_processes.is_empty() {
                    return;
                }
                let last = self.filtered_processes.len() - 1;
                self.process_table_state.select(Some(last));
                self.selected_pid = Some(self.processes[self.filtered_processes[last]].pid);
            }
        }
    }

    fn refresh_active_filter(&mut self) {
        match self.active_tab {
            ActiveTab::Logs => self.refresh_after_filter_change(),
            ActiveTab::Processes => self.filter_processes_preserving_selection(),
        }
    }

    fn rebuild_processes(&mut self) {
        self.processes.clear();
        for (pid, proc_info) in self.sys.processes() {
            self.processes.push(ProcessRow {
                pid: pid.as_u32(),
                name: proc_info.name().to_string_lossy().into_owned(),
                status: format!("{:?}", proc_info.status()),
                cpu_percent: proc_info.cpu_usage(),
                memory_bytes: proc_info.memory(),
                virtual_bytes: proc_info.virtual_memory(),
                exe: proc_info
                    .exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            });
        }
        self.sort_and_filter_processes();
    }

    fn sort_and_filter_processes(&mut self) {
        match self.process_sort {
            ProcessSort::Cpu => {
                self.processes.sort_by(|a, b| {
                    b.cpu_percent
                        .total_cmp(&a.cpu_percent)
                        .then_with(|| b.memory_bytes.cmp(&a.memory_bytes))
                        .then_with(|| a.pid.cmp(&b.pid))
                });
            }
            ProcessSort::Memory => {
                self.processes.sort_by(|a, b| {
                    b.memory_bytes
                        .cmp(&a.memory_bytes)
                        .then_with(|| b.cpu_percent.total_cmp(&a.cpu_percent))
                        .then_with(|| a.pid.cmp(&b.pid))
                });
            }
        }
        self.filter_processes_preserving_selection();
    }

    fn filter_processes_preserving_selection(&mut self) {
        let prev_pos = self.process_table_state.selected().unwrap_or(0);
        let query_lower = self.search_query.to_ascii_lowercase();
        self.filtered_processes.clear();

        for (idx, proc_row) in self.processes.iter().enumerate() {
            if query_lower.is_empty()
                || proc_row.name.to_ascii_lowercase().contains(&query_lower)
                || proc_row.pid.to_string().contains(&query_lower)
            {
                self.filtered_processes.push(idx);
            }
        }

        if self.filtered_processes.is_empty() {
            self.selected_pid = None;
            self.process_table_state.select(None);
            *self.process_table_state.offset_mut() = 0;
            return;
        }

        if let Some(target_pid) = self.selected_pid {
            if let Some(pos) = self
                .filtered_processes
                .iter()
                .position(|&idx| self.processes[idx].pid == target_pid)
            {
                self.process_table_state.select(Some(pos));
                return;
            }
        }

        let clamped = prev_pos.min(self.filtered_processes.len() - 1);
        self.selected_pid = Some(self.processes[self.filtered_processes[clamped]].pid);
        self.process_table_state.select(Some(clamped));
    }

    fn rebuild_filtered_indices(&mut self) {
        let query_lower = self.search_query.to_ascii_lowercase();
        self.filtered_indices.clear();
        for (idx, entry) in self.entries.iter().enumerate() {
            if Self::entry_matches(entry, self.filters, &query_lower) {
                self.filtered_indices.push(idx);
            }
        }
    }

    fn refresh_after_ingest(&mut self, evicted_matching: usize) {
        self.rebuild_filtered_indices();

        if self.filtered_indices.is_empty() {
            self.selected_id = None;
            self.table_state.select(None);
            *self.table_state.offset_mut() = 0;
            return;
        }

        if !self.paused {
            let last = self.filtered_indices.len() - 1;
            self.selected_id = Some(self.entries[self.filtered_indices[last]].id);
            self.table_state.select(Some(last));
            return;
        }

        if let Some(target_id) = self.selected_id {
            if let Some(new_pos) = self
                .filtered_indices
                .iter()
                .position(|&idx| self.entries[idx].id == target_id)
            {
                let old_offset = self.table_state.offset();
                *self.table_state.offset_mut() = old_offset.saturating_sub(evicted_matching);
                self.table_state.select(Some(new_pos));
                return;
            }
        }

        self.selected_id = Some(self.entries[self.filtered_indices[0]].id);
        self.table_state.select(Some(0));
        *self.table_state.offset_mut() = 0;
    }

    fn refresh_after_filter_change(&mut self) {
        let prev_pos = self.table_state.selected().unwrap_or(0);
        self.rebuild_filtered_indices();

        if self.filtered_indices.is_empty() {
            self.selected_id = None;
            self.table_state.select(None);
            *self.table_state.offset_mut() = 0;
            return;
        }

        if let Some(target_id) = self.selected_id {
            if let Some(pos) = self
                .filtered_indices
                .iter()
                .position(|&idx| self.entries[idx].id == target_id)
            {
                self.table_state.select(Some(pos));
                return;
            }
        }

        let next_pos = if self.paused {
            prev_pos.min(self.filtered_indices.len() - 1)
        } else {
            self.filtered_indices.len() - 1
        };
        self.selected_id = Some(self.entries[self.filtered_indices[next_pos]].id);
        self.table_state.select(Some(next_pos));
    }

    fn entry_matches(entry: &LogEntry, filters: LevelFilters, query_lower: &str) -> bool {
        if !filters.is_enabled(entry.level) {
            return false;
        }
        if query_lower.is_empty() {
            return true;
        }
        entry.raw.to_ascii_lowercase().contains(query_lower)
    }
}

fn read_primary_disk(disks: &Disks) -> (u64, u64) {
    let root = disks
        .iter()
        .find(|d| d.mount_point() == Path::new("/"))
        .or_else(|| disks.iter().next());
    if let Some(disk) = root {
        let total = disk.total_space();
        let avail = disk.available_space();
        (total.saturating_sub(avail), total)
    } else {
        (0, 0)
    }
}

pub fn spawn_ingestion(
    mode: AppMode,
    tx: Sender<LogEntry>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || match mode {
        AppMode::Demo => run_demo_generator(&tx, &stop),
        AppMode::System => run_system_stream(&tx, stop),
        AppMode::File(path) => run_file_tailer(path, &tx, &stop),
    })
}

fn run_system_stream(tx: &Sender<LogEntry>, stop: Arc<AtomicBool>) {
    let mut next_id: u64 = 1;
    let emitted_any = Arc::new(AtomicBool::new(false));

    if let Ok(mut child) = Command::new("/usr/bin/log")
        .args(["stream", "--style", "ndjson", "--level", "info"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(stdout) = child.stdout.take() {
            let tx_reader = tx.clone();
            let stop_reader = Arc::clone(&stop);
            let emitted_flag = Arc::clone(&emitted_any);
            let reader = std::thread::spawn(move || {
                let mut id: u64 = 1;
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if stop_reader.load(Ordering::Relaxed) {
                        break;
                    }
                    let trimmed = line.trim();
                    if !trimmed.starts_with('{') {
                        continue;
                    }
                    emitted_flag.store(true, Ordering::Relaxed);
                    let entry = parse_log_line(id, trimmed);
                    id += 1;
                    if tx_reader.send(entry).is_err() {
                        break;
                    }
                }
                id
            });

            loop {
                if stop.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return;
                }
                if let Ok(Some(_)) = child.try_wait() {
                    if let Ok(last_id) = reader.join() {
                        next_id = last_id;
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        } else {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    if stop.load(Ordering::Relaxed) {
        return;
    }

    if !emitted_any.load(Ordering::Relaxed) {
        run_sysinfo_log_fallback(tx, &stop, next_id);
    }
}

fn run_sysinfo_log_fallback(tx: &Sender<LogEntry>, stop: &AtomicBool, mut next_id: u64) {
    let Ok(mut sys) = System::new_with_specifics(
        RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_usage()),
    ) else {
        return;
    };
    sys.refresh_memory();
    let _ = sleep_or_stop(stop, sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        let cpu = sys.global_cpu_usage();
        let mem_used_gb = sys.used_memory() as f64 / 1_073_741_824.0;
        let mem_total_gb = sys.total_memory() as f64 / 1_073_741_824.0;
        let load_one = System::load_average().map(|l| l.one).unwrap_or(0.0);
        let top_proc = sys
            .processes()
            .values()
            .max_by(|a, b| a.cpu_usage().total_cmp(&b.cpu_usage()));

        let level = if cpu >= 85.0 {
            "WARN"
        } else {
            "INFO"
        };
        let top_desc = top_proc
            .map(|p| {
                format!(
                    "{} ({:.1}% CPU)",
                    p.name().to_string_lossy(),
                    p.cpu_usage()
                )
            })
            .unwrap_or_else(|| "none".to_string());

        let ts = Local::now().to_rfc3339_opts(SecondsFormat::Millis, false);
        let msg = format!(
            "CPU {:.1}% | RAM {mem_used_gb:.1}/{mem_total_gb:.1} GB | Load {load_one:.2} | Top: {top_desc}",
            cpu
        );
        let line = json!({
            "timestamp": ts,
            "level": level,
            "service": "host-monitor",
            "msg": msg,
            "cpu_percent": (cpu * 10.0).round() / 10.0,
            "load_1m": load_one
        })
        .to_string();

        let entry = parse_log_line(next_id, &line);
        next_id += 1;
        if tx.send(entry).is_err() || sleep_or_stop(stop, Duration::from_secs(2)) {
            return;
        }
    }
}

fn run_file_tailer(path: PathBuf, tx: &Sender<LogEntry>, stop: &AtomicBool) {
    let mut pos: u64 = 0;
    let mut partial: Vec<u8> = Vec::new();
    let mut skip_partial_first = false;
    let mut next_id: u64 = 1;

    if let Ok(mut file) = File::open(&path) {
        if let Ok(meta) = file.metadata() {
            let len = meta.len();
            if len > TAIL_INIT_BYTES {
                pos = len - TAIL_INIT_BYTES;
                skip_partial_first = true;
            }
            if !read_new_bytes(
                &mut file,
                &mut pos,
                &mut partial,
                &mut skip_partial_first,
                &mut next_id,
                tx,
            ) {
                return;
            }
        }
    }

    while !sleep_or_stop(stop, TAIL_POLL_INTERVAL) {
        let Ok(mut file) = File::open(&path) else {
            continue;
        };
        let Ok(meta) = file.metadata() else {
            continue;
        };
        let len = meta.len();
        if len < pos {
            pos = 0;
            partial.clear();
            skip_partial_first = false;
        }
        if len > pos
            && !read_new_bytes(
                &mut file,
                &mut pos,
                &mut partial,
                &mut skip_partial_first,
                &mut next_id,
                tx,
            )
        {
            return;
        }
    }
}

fn read_new_bytes(
    file: &mut File,
    pos: &mut u64,
    partial: &mut Vec<u8>,
    skip_partial_first: &mut bool,
    next_id: &mut u64,
    tx: &Sender<LogEntry>,
) -> bool {
    if file.seek(SeekFrom::Start(*pos)).is_err() {
        return true;
    }
    let mut buf = Vec::new();
    let Ok(read_bytes) = file.read_to_end(&mut buf) else {
        return true;
    };
    if read_bytes == 0 {
        return true;
    }

    *pos += read_bytes as u64;
    if *skip_partial_first {
        if let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            partial.extend_from_slice(&buf[nl + 1..]);
            *skip_partial_first = false;
        }
    } else {
        partial.extend_from_slice(&buf);
    }

    emit_complete_lines(partial, next_id, tx)
}

fn emit_complete_lines(buf: &mut Vec<u8>, next_id: &mut u64, tx: &Sender<LogEntry>) -> bool {
    let mut start = 0;
    while let Some(rel_nl) = buf[start..].iter().position(|&b| b == b'\n') {
        let nl = start + rel_nl;
        let mut line_bytes = &buf[start..nl];
        if let Some(stripped) = line_bytes.strip_suffix(b"\r") {
            line_bytes = stripped;
        }
        if !line_bytes.is_empty() {
            let text = String::from_utf8_lossy(line_bytes);
            let entry = parse_log_line(*next_id, &text);
            *next_id += 1;
            if tx.send(entry).is_err() {
                return false;
            }
        }
        start = nl + 1;
    }
    if start > 0 {
        buf.drain(..start);
    }
    true
}

fn run_demo_generator(tx: &Sender<LogEntry>, stop: &AtomicBool) {
    let mut rng = rand::rng();
    let mut next_id: u64 = 1;

    for _ in 0..18 {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let line = generate_demo_line(&mut rng);
        let entry = parse_log_line(next_id, &line);
        next_id += 1;
        if tx.send(entry).is_err() {
            return;
        }
    }

    let mut burst_remaining: u32 = 0;

    loop {
        if burst_remaining == 0 && rng.random_bool(0.08) {
            burst_remaining = rng.random_range(16..=42);
        }

        let delay = if burst_remaining > 0 {
            burst_remaining -= 1;
            Duration::from_millis(rng.random_range(15..=65))
        } else {
            Duration::from_millis(rng.random_range(280..=850))
        };

        if sleep_or_stop(stop, delay) {
            return;
        }

        let line = generate_demo_line(&mut rng);
        let entry = parse_log_line(next_id, &line);
        next_id += 1;
        if tx.send(entry).is_err() {
            return;
        }
    }
}

fn generate_demo_line(rng: &mut impl Rng) -> String {
    const SERVICES: [&str; 6] = [
        "api-gateway",
        "auth-service",
        "billing-worker",
        "postgres-proxy",
        "cache-cluster",
        "search-index",
    ];
    const ENDPOINTS: [&str; 5] = [
        "GET /v1/users/me",
        "POST /v1/orders",
        "GET /v1/invoices",
        "POST /v1/auth/token",
        "GET /v1/catalog/search",
    ];

    let roll: u8 = rng.random_range(0..100);
    let level = if roll < 60 {
        LogLevel::Info
    } else if roll < 80 {
        LogLevel::Debug
    } else if roll < 93 {
        LogLevel::Warn
    } else {
        LogLevel::Error
    };

    let service = SERVICES[rng.random_range(0..SERVICES.len())];
    let req_id = format!("req_{:08x}", rng.random_range(0x1000_0000u32..=0xffff_ffffu32));
    let latency_ms = match level {
        LogLevel::Info => rng.random_range(4..=145),
        LogLevel::Debug => rng.random_range(1..=25),
        LogLevel::Warn => rng.random_range(220..=950),
        LogLevel::Error => rng.random_range(850..=3200),
    };

    let (message, status_opt) = match level {
        LogLevel::Info => match rng.random_range(0..4) {
            0 => {
                let ep = ENDPOINTS[rng.random_range(0..ENDPOINTS.len())];
                let status = if ep.starts_with("POST") { 201 } else { 200 };
                (format!("{ep} status={status} latency={latency_ms}ms"), Some(status))
            }
            1 => {
                let rows = rng.random_range(1..=120);
                (
                    format!("DB query completed table=orders rows={rows} duration={latency_ms}ms"),
                    None,
                )
            }
            2 => {
                let ttl = rng.random_range(60..=3600);
                (
                    format!("Cache HIT key=session:{req_id} ttl={ttl}s rtt={latency_ms}ms"),
                    None,
                )
            }
            _ => {
                let batch = rng.random_range(1000..=9999);
                (
                    format!("Settled webhook batch #{batch} delivered=32 duration={latency_ms}ms"),
                    None,
                )
            }
        },
        LogLevel::Debug => match rng.random_range(0..3) {
            0 => (
                format!(
                    "SQL prepared stmt=fetch_ledger rows=12 elapsed={latency_ms}ms pool=primary"
                ),
                None,
            ),
            1 => (
                format!("Cache pipeline flushed cmds=6 rtt={latency_ms}ms shard=2"),
                None,
            ),
            _ => (
                format!("JWT claims validated sub=usr_{} scope=read:logs", rng.random_range(100..=999)),
                None,
            ),
        },
        LogLevel::Warn => match rng.random_range(0..3) {
            0 => (
                format!("Slow DB query exceeded threshold table=events duration={latency_ms}ms"),
                None,
            ),
            1 => (
                format!(
                    "Retrying upstream call attempt=2/3 backoff=250ms target=stripe-api duration={latency_ms}ms"
                ),
                Some(429),
            ),
            _ => (
                format!("Cache MISS key=catalog:featured falling back to replica latency={latency_ms}ms"),
                None,
            ),
        },
        LogLevel::Error => match rng.random_range(0..3) {
            0 => (
                format!(
                    "Authentication failure reason=\"invalid signature\" status=401 ip=10.24.{}.{}",
                    rng.random_range(1..=254),
                    rng.random_range(1..=254)
                ),
                Some(401),
            ),
            1 => (
                format!(
                    "Upstream request timeout after {latency_ms}ms target=payment-gateway status=504"
                ),
                Some(504),
            ),
            _ => (
                "Database deadlock on table=invoices retries_exhausted=3 err=\"serialization failure\""
                    .to_string(),
                Some(500),
            ),
        },
    };

    let ts = Local::now().to_rfc3339_opts(SecondsFormat::Millis, false);
    let level_str = level.short_name();

    if rng.random_bool(0.30) {
        let mut obj = json!({
            "timestamp": ts,
            "level": level_str,
            "service": service,
            "request_id": req_id,
            "msg": message,
            "latency_ms": latency_ms
        });
        if let (Some(status), Some(map)) = (status_opt, obj.as_object_mut()) {
            map.insert("status".to_string(), json!(status));
        }
        obj.to_string()
    } else {
        format!("{ts} {level_str:<5} [{service}] {req_id} {message}")
    }
}

fn sleep_or_stop(stop: &AtomicBool, duration: Duration) -> bool {
    let slice = Duration::from_millis(25);
    let mut remaining = duration;
    while remaining > Duration::ZERO {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        let step = remaining.min(slice);
        std::thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    stop.load(Ordering::Relaxed)
}

#[derive(Deserialize)]
struct JsonLogFields {
    #[serde(default)]
    level: Option<Value>,
    #[serde(default)]
    severity: Option<Value>,
    #[serde(default)]
    lvl: Option<Value>,
    #[serde(rename = "messageType", default)]
    message_type: Option<Value>,
    #[serde(default)]
    msg: Option<Value>,
    #[serde(default)]
    message: Option<Value>,
    #[serde(rename = "eventMessage", default)]
    event_message: Option<Value>,
    #[serde(rename = "processImagePath", default)]
    process_image_path: Option<Value>,
    #[serde(default)]
    timestamp: Option<Value>,
    #[serde(default)]
    time: Option<Value>,
    #[serde(default)]
    ts: Option<Value>,
}

pub fn parse_log_line(id: u64, raw_line: &str) -> LogEntry {
    let raw = raw_line.trim_end_matches(['\r', '\n']).to_string();
    let trimmed = raw.trim();

    if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
        if let Ok(fields) = serde_json::from_value::<JsonLogFields>(val.clone()) {
            let level = fields
                .level
                .as_ref()
                .or(fields.severity.as_ref())
                .or(fields.lvl.as_ref())
                .or(fields.message_type.as_ref())
                .and_then(value_to_level)
                .unwrap_or_else(|| detect_plain_level(trimmed));

            let base_msg = fields
                .msg
                .as_ref()
                .or(fields.message.as_ref())
                .or(fields.event_message.as_ref())
                .map(json_value_to_string)
                .filter(|s| !s.is_empty());

            let message = match (base_msg, fields.process_image_path.as_ref().and_then(|v| v.as_str())) {
                (Some(msg), Some(img)) if fields.msg.is_none() && fields.message.is_none() => {
                    let proc_name = Path::new(img)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(img);
                    format!("[{proc_name}] {msg}")
                }
                (Some(msg), _) => msg,
                (None, _) => trimmed.to_string(),
            };

            let timestamp = fields
                .timestamp
                .as_ref()
                .or(fields.time.as_ref())
                .or(fields.ts.as_ref())
                .and_then(parse_json_timestamp)
                .unwrap_or_else(current_time_str);

            return LogEntry {
                id,
                timestamp,
                level,
                message,
                raw,
                json: Some(val),
            };
        }

        let timestamp = extract_timestamp_from_str(trimmed).unwrap_or_else(current_time_str);
        let level = detect_plain_level(trimmed);
        return LogEntry {
            id,
            timestamp,
            level,
            message: trimmed.to_string(),
            raw,
            json: Some(val),
        };
    }

    let timestamp = extract_timestamp_from_str(trimmed).unwrap_or_else(current_time_str);
    let level = detect_plain_level(trimmed);
    let message = clean_plain_message(trimmed);

    LogEntry {
        id,
        timestamp,
        level,
        message,
        raw,
        json: None,
    }
}

fn value_to_level(v: &Value) -> Option<LogLevel> {
    if let Some(s) = v.as_str() {
        return level_from_str(s);
    }
    if let Some(n) = v.as_i64() {
        return match n {
            50.. => Some(LogLevel::Error),
            40..=49 => Some(LogLevel::Warn),
            30..=39 => Some(LogLevel::Info),
            10..=29 => Some(LogLevel::Debug),
            _ => None,
        };
    }
    None
}

fn level_from_str(s: &str) -> Option<LogLevel> {
    match s.trim().to_ascii_uppercase().as_str() {
        "ERROR" | "ERR" | "FAULT" => Some(LogLevel::Error),
        "WARN" | "WARNING" => Some(LogLevel::Warn),
        "INFO" | "DEFAULT" => Some(LogLevel::Info),
        "DEBUG" | "TRACE" => Some(LogLevel::Debug),
        _ => None,
    }
}

fn json_value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn parse_json_timestamp(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return extract_timestamp_from_str(s);
    }
    if let Some(n) = v.as_i64() {
        let dt = if n > 1_000_000_000_000 {
            DateTime::from_timestamp_millis(n)
        } else {
            DateTime::from_timestamp(n, 0)
        }?;
        return Some(dt.with_timezone(&Local).format("%H:%M:%S").to_string());
    }
    if let Some(f) = v.as_f64() {
        let secs = if f > 1e12 {
            (f / 1000.0) as i64
        } else {
            f as i64
        };
        let dt = DateTime::from_timestamp(secs, 0)?;
        return Some(dt.with_timezone(&Local).format("%H:%M:%S").to_string());
    }
    None
}

fn current_time_str() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

fn extract_timestamp_from_str(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Local).format("%H:%M:%S").to_string());
    }

    if let Some(first_tok) = trimmed.split_whitespace().next() {
        let clean_tok = first_tok.trim_matches(|c| c == '[' || c == ']' || c == ',');
        if let Ok(dt) = DateTime::parse_from_rfc3339(clean_tok) {
            return Some(dt.with_timezone(&Local).format("%H:%M:%S").to_string());
        }
    }

    let limit = trimmed.len().min(48);
    let prefix = &trimmed.as_bytes()[..limit];
    for (i, chunk) in prefix.windows(8).enumerate() {
        let matches_pattern = chunk[0].is_ascii_digit()
            && chunk[1].is_ascii_digit()
            && chunk[2] == b':'
            && chunk[3].is_ascii_digit()
            && chunk[4].is_ascii_digit()
            && chunk[5] == b':'
            && chunk[6].is_ascii_digit()
            && chunk[7].is_ascii_digit();

        if !matches_pattern {
            continue;
        }

        let left_ok = i == 0
            || (!prefix[i - 1].is_ascii_alphanumeric() && prefix[i - 1] != b':')
            || matches!(prefix[i - 1], b'T' | b't');
        let right_ok = i + 8 == prefix.len()
            || (!prefix[i + 8].is_ascii_alphanumeric() && prefix[i + 8] != b':')
            || matches!(prefix[i + 8], b'Z' | b'z');

        if !left_ok || !right_ok {
            continue;
        }

        let hh = (chunk[0] - b'0') * 10 + (chunk[1] - b'0');
        let mm = (chunk[3] - b'0') * 10 + (chunk[4] - b'0');
        let ss = (chunk[6] - b'0') * 10 + (chunk[7] - b'0');
        if hh < 24 && mm < 60 && ss < 60 {
            return Some(format!("{hh:02}:{mm:02}:{ss:02}"));
        }
    }

    None
}

fn detect_plain_level(text: &str) -> LogLevel {
    for tok in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        match tok {
            "ERROR" | "ERR" => return LogLevel::Error,
            "WARN" | "WARNING" => return LogLevel::Warn,
            "INFO" => return LogLevel::Info,
            "DEBUG" | "TRACE" => return LogLevel::Debug,
            _ => {}
        }
    }

    let mut found_warn = false;
    let mut found_debug = false;
    for tok in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        if tok.eq_ignore_ascii_case("error") || tok.eq_ignore_ascii_case("err") {
            return LogLevel::Error;
        }
        if tok.eq_ignore_ascii_case("warn") || tok.eq_ignore_ascii_case("warning") {
            found_warn = true;
        } else if tok.eq_ignore_ascii_case("debug") || tok.eq_ignore_ascii_case("trace") {
            found_debug = true;
        }
    }

    if found_warn {
        LogLevel::Warn
    } else if found_debug {
        LogLevel::Debug
    } else {
        LogLevel::Info
    }
}

fn clean_plain_message(trimmed: &str) -> String {
    let mut rest = trimmed;

    if rest.starts_with('[') {
        if let Some(end) = rest.find(']') {
            let inside = &rest[1..end];
            if extract_timestamp_from_str(inside).is_some() {
                rest = rest[end + 1..].trim_start();
            }
        }
    } else if let Some(first) = rest.split_whitespace().next() {
        if extract_timestamp_from_str(first).is_some() {
            rest = rest[first.len()..].trim_start();
        } else if is_iso_date_prefix(first) {
            let after_date = rest[first.len()..].trim_start();
            if let Some(next_tok) = after_date.split_whitespace().next() {
                if extract_timestamp_from_str(next_tok).is_some() {
                    rest = after_date[next_tok.len()..].trim_start();
                }
            }
        }
    }

    if let Some(first) = rest.split_whitespace().next() {
        let stripped = first.trim_matches(|c| c == '[' || c == ']' || c == ':');
        if level_from_str(stripped).is_some() {
            rest = rest[first.len()..].trim_start();
            if let Some(after_colon) = rest.strip_prefix(':') {
                rest = after_colon.trim_start();
            }
        }
    }

    if rest.is_empty() {
        trimmed.to_string()
    } else {
        rest.to_string()
    }
}

fn is_iso_date_prefix(tok: &str) -> bool {
    let b = tok.as_bytes();
    b.len() == 10
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn parses_json_log_variants() {
        let line1 = r#"{"timestamp":"2026-11-18T12:34:56Z","level":"error","msg":"db timeout"}"#;
        let entry1 = parse_log_line(1, line1);
        assert_eq!(entry1.level, LogLevel::Error);
        assert_eq!(entry1.message, "db timeout");
        assert!(entry1.json.is_some());

        let line2 = r#"{"ts":1700000000,"severity":"WARNING","message":"cache miss"}"#;
        let entry2 = parse_log_line(2, line2);
        assert_eq!(entry2.level, LogLevel::Warn);
        assert_eq!(entry2.message, "cache miss");
        assert!(entry2.json.is_some());

        let line3 = r#"{"time":"10:20:30","lvl":"trace","msg":"step 1"}"#;
        let entry3 = parse_log_line(3, line3);
        assert_eq!(entry3.level, LogLevel::Debug);
        assert_eq!(entry3.timestamp, "10:20:30");

        let line4 = r#"{"timestamp":"2026-11-18 14:32:10.123456+0800","messageType":"Error","processImagePath":"/usr/libexec/WindowServer","eventMessage":"display link timeout"}"#;
        let entry4 = parse_log_line(4, line4);
        assert_eq!(entry4.level, LogLevel::Error);
        assert_eq!(entry4.timestamp, "14:32:10");
        assert_eq!(entry4.message, "[WindowServer] display link timeout");
    }

    #[test]
    fn detects_plain_text_levels_and_defaults() {
        assert_eq!(
            parse_log_line(1, "10:00:00 ERR failed to connect").level,
            LogLevel::Error
        );
        assert_eq!(
            parse_log_line(2, "[warn] slow query detected").level,
            LogLevel::Warn
        );
        assert_eq!(
            parse_log_line(3, "2026-11-18T10:00:00Z INFO 0 errors reported").level,
            LogLevel::Info
        );
        assert_eq!(
            parse_log_line(4, "TRACE executing query plan").level,
            LogLevel::Debug
        );
        assert_eq!(
            parse_log_line(5, "service started normally").level,
            LogLevel::Info
        );
    }

    #[test]
    fn filters_by_level_and_case_insensitive_search() {
        let mut app = App::new(AppMode::Demo).expect("app init");
        let (tx, rx) = mpsc::channel();
        tx.send(parse_log_line(1, "10:00:01 INFO [api] GET /health 200"))
            .expect("send");
        tx.send(parse_log_line(2, "10:00:02 ERROR [auth] Invalid Token"))
            .expect("send");
        tx.send(parse_log_line(3, "10:00:03 WARN [db] slow query"))
            .expect("send");
        app.drain_incoming(&rx);

        assert_eq!(app.filtered_indices.len(), 3);

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for ch in ['t', 'o', 'k', 'e', 'n'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.selected_entry().expect("selected").id, 2);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.filtered_indices.len(), 3);

        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    fn anchors_selection_when_paused() {
        let mut app = App::new(AppMode::Demo).expect("app init");
        let (tx, rx) = mpsc::channel();
        tx.send(parse_log_line(1, "10:00:01 INFO first")).expect("send");
        tx.send(parse_log_line(2, "10:00:02 INFO second")).expect("send");
        app.drain_incoming(&rx);
        assert_eq!(app.selected_id, Some(2));

        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert!(app.paused);
        assert_eq!(app.selected_id, Some(1));
        assert_eq!(app.table_state.selected(), Some(0));

        tx.send(parse_log_line(3, "10:00:03 ERROR third")).expect("send");
        tx.send(parse_log_line(4, "10:00:04 WARN fourth")).expect("send");
        app.drain_incoming(&rx);

        assert!(app.paused);
        assert_eq!(app.selected_id, Some(1));
        assert_eq!(app.table_state.selected(), Some(0));

        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert!(!app.paused);
        assert_eq!(app.selected_id, Some(4));
    }

    #[test]
    fn system_mode_starts_on_processes_and_switches_tabs() {
        let mut app = App::new(AppMode::System).expect("system app init");
        assert_eq!(app.active_tab, ActiveTab::Processes);
        assert!(!app.processes.is_empty());

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Logs);

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Processes);
    }
}
