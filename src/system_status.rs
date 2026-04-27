// MIT License
// Copyright (c) 2026 Cedric Gegout

use sysinfo::{System, ProcessRefreshKind, CpuRefreshKind, RefreshKind, MemoryRefreshKind};

/// Holds a snapshot of current machine resource usage.
#[derive(Debug, Clone)]
pub struct SystemStatus {
    pub cpu_usage: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub disk_used_gb: u64,
    pub disk_total_gb: u64,
    pub uptime_seconds: u64,
    /// Top processes as (name, cpu_percent, mem_mb).
    pub top_processes: Vec<(String, f32, u64)>,
}

/// Collects current machine metrics into a `SystemStatus`.
pub fn get_system_status() -> SystemStatus {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
            .with_processes(ProcessRefreshKind::everything()),
    );

    // Sleep briefly so sysinfo can compute a meaningful CPU diff.
    std::thread::sleep(std::time::Duration::from_millis(300));
    sys.refresh_all();

    let cpu_usage = sys.global_cpu_info().cpu_usage();
    let memory_used_mb = sys.used_memory() / 1024 / 1024;
    let memory_total_mb = sys.total_memory() / 1024 / 1024;
    let uptime_seconds = System::uptime();

    // Walk root disk only.
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

    SystemStatus {
        cpu_usage,
        memory_used_mb,
        memory_total_mb,
        disk_used_gb,
        disk_total_gb,
        uptime_seconds,
        top_processes,
    }
}
