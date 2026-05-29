// MIT License
// Copyright (c) 2026 Cedric Gegout

use std::fs;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, RefreshKind, System};

// ─── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub name: String,
    pub cpu_pct: f32,
    pub mem_mb: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Warning,
    Critical,
}

impl HealthStatus {
    pub fn emoji(&self) -> &'static str {
        match self {
            HealthStatus::Healthy  => "✅",
            HealthStatus::Warning  => "⚠️",
            HealthStatus::Critical => "🔴",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            HealthStatus::Healthy  => "Healthy",
            HealthStatus::Warning  => "Warning",
            HealthStatus::Critical => "Critical",
        }
    }
}

/// Full non-privileged machine health snapshot.
#[derive(Debug, Clone)]
pub struct SystemStatus {
    // Identity
    pub hostname: String,
    pub os_name: String,
    pub current_user: String,
    // Resources
    pub cpu_usage: f32,
    pub cpu_count: usize,
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub swap_used_mb: u64,
    pub swap_total_mb: u64,
    pub disk_used_gb: u64,
    pub disk_total_gb: u64,
    pub uptime_seconds: u64,
    // Processes
    pub total_processes: usize,
    pub top_cpu_processes: Vec<ProcessInfo>,
    pub top_mem_processes: Vec<ProcessInfo>,
    // Log signals (best-effort, mixed sources, may be empty)
    pub log_signals: Vec<String>,
    // Health assessment
    pub health: HealthStatus,
    pub findings: Vec<String>,
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn get_system_status() -> SystemStatus {
    tracing::info!("Starting full system status collection...");
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
            .with_processes(ProcessRefreshKind::everything()),
    );
    // Brief pause so sysinfo can compute an inter-sample CPU delta.
    std::thread::sleep(std::time::Duration::from_millis(300));
    sys.refresh_all();

    let cpu_usage       = sys.global_cpu_info().cpu_usage();
    let cpu_count       = sys.cpus().len();
    let memory_used_mb  = sys.used_memory()  / 1_048_576;
    let memory_total_mb = sys.total_memory() / 1_048_576;
    let swap_used_mb    = sys.used_swap()    / 1_048_576;
    let swap_total_mb   = sys.total_swap()   / 1_048_576;
    let uptime_seconds  = System::uptime();
    let hostname        = System::host_name().unwrap_or_else(|| "unknown".to_string());
    let os_name         = System::long_os_version().unwrap_or_else(|| "unknown".to_string());
    let current_user    = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    // Load averages from sysinfo.
    let load = System::load_average();
    let (load_1m, load_5m, load_15m) = (load.one, load.five, load.fifteen);

    // Root disk via sysinfo Disks.
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut disk_used_gb  = 0u64;
    let mut disk_total_gb = 0u64;
    for disk in disks.iter() {
        if disk.mount_point().to_string_lossy() == "/" {
            disk_total_gb = disk.total_space()       / 1_073_741_824;
            disk_used_gb  = (disk.total_space() - disk.available_space()) / 1_073_741_824;
            break;
        }
    }

    // Process lists.
    let total_processes = sys.processes().len();
    let mut procs: Vec<_> = sys.processes().values().collect();

    procs.sort_by(|a, b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
    let top_cpu_processes: Vec<ProcessInfo> = procs.iter().take(3).map(|p| ProcessInfo {
        name: p.name().to_string(), cpu_pct: p.cpu_usage(), mem_mb: p.memory() / 1_048_576,
    }).collect();

    procs.sort_by(|a, b| b.memory().cmp(&a.memory()));
    let top_mem_processes: Vec<ProcessInfo> = procs.iter().take(3).map(|p| ProcessInfo {
        name: p.name().to_string(), cpu_pct: p.cpu_usage(), mem_mb: p.memory() / 1_048_576,
    }).collect();

    // Best-effort log signals.
    let log_signals = collect_log_signals();

    // Derive health classification and findings.
    let (health, findings) = assess_health(
        cpu_usage, cpu_count,
        memory_used_mb, memory_total_mb,
        disk_used_gb, disk_total_gb,
        load_1m, &log_signals, uptime_seconds,
    );

    SystemStatus {
        hostname, os_name, current_user,
        cpu_usage, cpu_count,
        load_1m, load_5m, load_15m,
        memory_used_mb, memory_total_mb,
        swap_used_mb, swap_total_mb,
        disk_used_gb, disk_total_gb,
        uptime_seconds,
        total_processes, top_cpu_processes, top_mem_processes,
        log_signals,
        health, findings,
    }
}

// ─── Log signal collection (all best-effort, non-privileged) ─────────────────

/// Reads up to `max_lines` error/warning lines from the tail of a text file.
/// Returns `None` if the file is unreadable or contains no relevant lines.
fn tail_filter(path: &str, max_lines: usize) -> Option<Vec<String>> {
    let content = fs::read_to_string(path).ok()?;
    let keywords = ["error", "warn", "crit", "fail", "panic", "oom", "killed", "segfault"];
    let filtered: Vec<String> = content
        .lines()
        .rev()
        .take(500)   // scan last 500 lines
        .filter(|l| { let lo = l.to_lowercase(); keywords.iter().any(|k| lo.contains(k)) })
        .take(max_lines)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter().rev().collect();
    if filtered.is_empty() { None } else { Some(filtered) }
}

/// Runs an external command and returns its stdout lines; silently returns `[]` on any failure.
fn run_cmd(cmd: &str, args: &[&str]) -> Vec<String> {
    match std::process::Command::new(cmd).args(args).output() {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines().filter(|l| !l.is_empty()).map(str::to_string).collect()
        }
        Ok(out) => {
            tracing::debug!("'{}' non-zero: {}", cmd, String::from_utf8_lossy(&out.stderr).trim());
            vec![]
        }
        Err(e) => { tracing::debug!("'{}' unavailable: {}", cmd, e); vec![] }
    }
}

/// Aggregates log/health signals from all readable non-privileged sources.
fn collect_log_signals() -> Vec<String> {
    let mut signals: Vec<String> = Vec::new();

    // System log files (readable by normal users on many distros).
    for (label, path) in &[
        ("syslog",  "/var/log/syslog"),
        ("kern",    "/var/log/kern.log"),
        ("auth",    "/var/log/auth.log"),
    ] {
        match tail_filter(path, 5) {
            Some(lines) => lines.into_iter().for_each(|l| signals.push(format!("[{}] {}", label, l))),
            None        => tracing::debug!("{} not readable or no relevant entries.", path),
        }
    }

    // User-accessible journalctl (no sudo required).
    for args in &[
        vec!["--user",   "--priority=warning", "--lines=8", "--no-pager", "--output=short-iso"],
        vec!["--system", "--priority=warning", "--lines=8", "--no-pager", "--output=short-iso"],
    ] {
        for l in run_cmd("journalctl", args) {
            signals.push(format!("[journal] {}", l));
        }
    }

    // Application's own log file (always owned by the current user).
    if let Some(lines) = tail_filter("mattermost-digest.out", 5) {
        for l in lines { signals.push(format!("[app] {}", l)); }
    }

    // /var/crash listing — readable without privilege on Ubuntu.
    if let Ok(entries) = fs::read_dir("/var/crash") {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    let age = std::time::SystemTime::now()
                        .duration_since(modified).unwrap_or_default();
                    if age.as_secs() < 7 * 86400 {
                        signals.push(format!("[crash] Recent crash file: {}",
                            entry.file_name().to_string_lossy()));
                    }
                }
            }
        }
    } else {
        tracing::debug!("/var/crash not readable.");
    }

    tracing::info!("Collected {} log signals from non-privileged sources.", signals.len());
    signals
}

// ─── Health assessment ────────────────────────────────────────────────────────

fn assess_health(
    cpu_usage: f32, cpu_count: usize,
    memory_used_mb: u64, memory_total_mb: u64,
    disk_used_gb: u64, disk_total_gb: u64,
    load_1m: f64,
    log_signals: &[String],
    uptime_seconds: u64,
) -> (HealthStatus, Vec<String>) {
    let mut status   = HealthStatus::Healthy;
    let mut findings: Vec<String> = Vec::new();

    let mem_pct  = if memory_total_mb > 0 { memory_used_mb  * 100 / memory_total_mb  } else { 0 };
    let disk_pct = if disk_total_gb   > 0 { disk_used_gb    * 100 / disk_total_gb    } else { 0 };
    let cpus     = cpu_count.max(1) as f64;

    // CPU thresholds.
    if cpu_usage > 85.0 {
        status = HealthStatus::Critical;
        findings.push(format!("CPU is critically high ({:.0}%)", cpu_usage));
    } else if cpu_usage > 65.0 {
        if status == HealthStatus::Healthy { status = HealthStatus::Warning; }
        findings.push(format!("CPU usage is elevated ({:.0}%)", cpu_usage));
    }

    // Memory thresholds.
    if mem_pct > 90 {
        status = HealthStatus::Critical;
        findings.push("Memory pressure is critical".to_string());
    } else if mem_pct > 80 {
        if status == HealthStatus::Healthy { status = HealthStatus::Warning; }
        findings.push("High memory usage".to_string());
    }

    // Disk thresholds.
    if disk_pct > 90 {
        status = HealthStatus::Critical;
        findings.push("Root disk is almost full".to_string());
    } else if disk_pct > 80 {
        if status == HealthStatus::Healthy { status = HealthStatus::Warning; }
        findings.push("Disk space on / is low".to_string());
    }

    // Load thresholds.
    if load_1m > cpus * 2.0 {
        status = HealthStatus::Critical;
        findings.push(format!("System load is critically high ({:.2})", load_1m));
    } else if load_1m > cpus {
        if status == HealthStatus::Healthy { status = HealthStatus::Warning; }
        findings.push(format!("Load is elevated ({:.2}, {} CPUs)", load_1m, cpu_count));
    }

    // Log signals.
    let has_errors = log_signals.iter().any(|l| {
        let lo = l.to_lowercase();
        lo.contains("error") || lo.contains("crit") || lo.contains("crash")
    });
    if has_errors {
        if status == HealthStatus::Healthy { status = HealthStatus::Warning; }
        findings.push("Recent errors or warnings in system logs".to_string());
    }

    // Recent reboot.
    if uptime_seconds < 600 {
        if status == HealthStatus::Healthy { status = HealthStatus::Warning; }
        findings.push(format!("Machine was recently rebooted (uptime {}m)", uptime_seconds / 60));
    }

    if findings.is_empty() {
        findings.push("No obvious issue detected".to_string());
        findings.push("System appears healthy".to_string());
    }

    (status, findings)
}
