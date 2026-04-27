// MIT License
// Copyright (c) 2026 Cedric Gegout

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, RefreshKind, System};

/// A snapshot of current machine resource usage and recent kernel log entries.
#[derive(Debug, Clone)]
pub struct SystemStatus {
    pub cpu_usage: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub disk_used_gb: u64,
    pub disk_total_gb: u64,
    pub uptime_seconds: u64,
    /// Top 2 processes as (name, cpu_percent, mem_mb).
    pub top_processes: Vec<(String, f32, u64)>,
    /// Last 20 warning-or-higher lines from journalctl, most-recent first.
    pub kernel_log_entries: Vec<String>,
}

/// Collects current machine metrics and kernel log entries into a `SystemStatus`.
pub fn get_system_status() -> SystemStatus {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
            .with_processes(ProcessRefreshKind::everything()),
    );

    // Brief sleep so sysinfo can compute a meaningful inter-sample CPU delta.
    std::thread::sleep(std::time::Duration::from_millis(300));
    sys.refresh_all();

    let cpu_usage = sys.global_cpu_info().cpu_usage();
    let memory_used_mb = sys.used_memory() / 1024 / 1024;
    let memory_total_mb = sys.total_memory() / 1024 / 1024;
    let uptime_seconds = System::uptime();

    // Collect root-partition disk usage.
    let disks_list = sysinfo::Disks::new_with_refreshed_list();
    let mut disk_used_gb = 0u64;
    let mut disk_total_gb = 0u64;
    for disk in disks_list.iter() {
        if disk.mount_point().to_string_lossy() == "/" {
            disk_total_gb = disk.total_space() / 1024 / 1024 / 1024;
            disk_used_gb = (disk.total_space() - disk.available_space()) / 1024 / 1024 / 1024;
            break;
        }
    }

    // Sort processes by CPU descending and take the top 2.
    let mut processes: Vec<_> = sys.processes().values().collect();
    processes.sort_by(|a, b| {
        b.cpu_usage()
            .partial_cmp(&a.cpu_usage())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_processes = processes
        .iter()
        .take(2)
        .map(|p| (p.name().to_string(), p.cpu_usage(), p.memory() / 1024 / 1024))
        .collect();

    // Fetch kernel log entries.
    let kernel_log_entries = get_kernel_log_entries();

    SystemStatus {
        cpu_usage,
        memory_used_mb,
        memory_total_mb,
        disk_used_gb,
        disk_total_gb,
        uptime_seconds,
        top_processes,
        kernel_log_entries,
    }
}

/// Fetches the last 20 warning-or-higher priority messages from `journalctl`.
/// Returns an empty list if journalctl is unavailable or fails.
pub fn get_kernel_log_entries() -> Vec<String> {
    tracing::info!("Fetching kernel/journal warning and error log entries...");
    let output = std::process::Command::new("journalctl")
        .args([
            "--priority=warning",  // warning, err, crit, alert, emerg
            "--lines=20",
            "--no-pager",
            "--output=short-iso",  // ISO8601 timestamps so Gemini can reason about age
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<String> = text
                .lines()
                .map(|l| l.to_string())
                .filter(|l| !l.is_empty())
                .collect();
            tracing::info!("Collected {} kernel log entries.", lines.len());
            lines
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::warn!("journalctl returned non-zero status: {}", stderr);
            vec![format!("journalctl error: {}", stderr.trim())]
        }
        Err(e) => {
            tracing::warn!("Could not run journalctl: {}", e);
            vec![format!("journalctl unavailable: {}", e)]
        }
    }
}
