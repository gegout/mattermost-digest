// MIT License
// Copyright (c) 2026 Cedric Gegout

use crate::system_status::SystemStatus;

/// Escapes HTML special characters for safe inclusion in Telegram HTML messages.
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Formats a comprehensive machine health status message for Telegram (HTML).
pub fn format_system_status(s: &SystemStatus) -> String {
    let uptime_d = s.uptime_seconds / 86400;
    let uptime_h = (s.uptime_seconds % 86400) / 3600;
    let uptime_m = (s.uptime_seconds % 3600) / 60;

    let mem_used_gb  = s.memory_used_mb  as f64 / 1024.0;
    let mem_total_gb = s.memory_total_mb as f64 / 1024.0;

    // Disk usage bar (10 cells).
    let disk_pct = if s.disk_total_gb > 0 { s.disk_used_gb * 10 / s.disk_total_gb } else { 0 } as usize;
    let disk_bar = format!("[{}{}]", "█".repeat(disk_pct), "░".repeat(10 - disk_pct));

    // Memory usage bar.
    let mem_pct = if s.memory_total_mb > 0 { s.memory_used_mb * 10 / s.memory_total_mb } else { 0 } as usize;
    let mem_bar = format!("[{}{}]", "█".repeat(mem_pct), "░".repeat(10 - mem_pct));

    let mut msg = String::new();

    // ── Header ──────────────────────────────────────────────────────────────
    msg.push_str(&format!(
        "📊 <b>Machine Status</b> — <code>{}</code>\n\
         🖥️ <b>OS:</b> {}\n\
         👤 <b>User:</b> {}\n\n",
        escape_html(&s.hostname),
        escape_html(&s.os_name),
        escape_html(&s.current_user),
    ));

    // ── Overall health ──────────────────────────────────────────────────────
    msg.push_str(&format!("<b>Overall:</b> {} <b>{}</b>\n\n", s.health.emoji(), s.health.label()));

    // ── Resources ───────────────────────────────────────────────────────────
    msg.push_str(&format!("🖥️ <b>CPU:</b> {:.1}%  ({} cores)\n", s.cpu_usage, s.cpu_count));
    msg.push_str(&format!(
        "🧠 <b>Memory:</b> <code>{}</code>  {:.1} / {:.1} GB\n",
        mem_bar, mem_used_gb, mem_total_gb,
    ));
    if s.swap_total_mb > 0 {
        msg.push_str(&format!("💱 <b>Swap:</b> {} / {} MB\n", s.swap_used_mb, s.swap_total_mb));
    }
    msg.push_str(&format!(
        "💾 <b>Disk /:</b> <code>{}</code>  {} / {} GB\n",
        disk_bar, s.disk_used_gb, s.disk_total_gb,
    ));
    msg.push_str(&format!(
        "📈 <b>Load:</b> {:.2}, {:.2}, {:.2}  ({} CPU{})\n",
        s.load_1m, s.load_5m, s.load_15m,
        s.cpu_count, if s.cpu_count == 1 { "" } else { "s" },
    ));
    msg.push_str(&format!(
        "⏱️ <b>Uptime:</b> {}d {}h {}m   🔢 <b>Processes:</b> {}\n\n",
        uptime_d, uptime_h, uptime_m, s.total_processes,
    ));

    // ── Findings ─────────────────────────────────────────────────────────────
    msg.push_str("🔎 <b>Findings</b>\n");
    for f in &s.findings {
        msg.push_str(&format!("• {}\n", escape_html(f)));
    }
    msg.push('\n');

    // ── Top CPU processes ─────────────────────────────────────────────────────
    msg.push_str("🔥 <b>Top CPU processes</b>\n");
    for p in &s.top_cpu_processes {
        msg.push_str(&format!("• <code>{}</code> — {:.1}%  ({} MB)\n",
            escape_html(&p.name), p.cpu_pct, p.mem_mb));
    }
    msg.push('\n');

    // ── Top memory processes ──────────────────────────────────────────────────
    msg.push_str("🧠 <b>Top memory processes</b>\n");
    for p in &s.top_mem_processes {
        msg.push_str(&format!("• <code>{}</code> — {} MB  ({:.1}% CPU)\n",
            escape_html(&p.name), p.mem_mb, p.cpu_pct));
    }

    msg
}

/// Formats an error message for Telegram HTML output.
pub fn format_error(err: &str) -> String {
    format!("❌ <b>Error:</b>\n<pre>{}</pre>", escape_html(err))
}

/// Formats a success message for Telegram HTML output.
pub fn format_success(msg: &str) -> String {
    format!("✅ <b>Success:</b> {}", escape_html(msg))
}
