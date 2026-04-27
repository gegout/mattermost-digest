// MIT License
// Copyright (c) 2026 Cedric Gegout

use crate::system_status::SystemStatus;

/// Escapes HTML characters for Telegram formatting.
pub fn escape_html(text: &str) -> String {
    text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
}

pub fn format_system_status(status: &SystemStatus) -> String {
    let uptime_hrs = status.uptime_seconds / 3600;
    let uptime_mins = (status.uptime_seconds % 3600) / 60;
    
    let mut msg = String::new();
    msg.push_str("🖥 <b>Machine Status</b>\n\n");
    msg.push_str(&format!("⚙️ <b>CPU:</b> {:.1}%\n", status.cpu_usage));
    msg.push_str(&format!("🧠 <b>Memory:</b> {} MB / {} MB\n", status.memory_used_mb, status.memory_total_mb));
    msg.push_str(&format!("💾 <b>Disk (/):</b> {} GB / {} GB\n", status.disk_used_gb, status.disk_total_gb));
    msg.push_str(&format!("⏱ <b>Uptime:</b> {}h {}m\n\n", uptime_hrs, uptime_mins));
    
    msg.push_str("🔥 <b>Top Processes:</b>\n");
    for (name, cpu, mem) in &status.top_processes {
        msg.push_str(&format!("• <code>{}</code>: {:.1}% CPU, {} MB\n", escape_html(name), cpu, mem));
    }
    
    msg
}

pub fn format_error(err: &str) -> String {
    format!("❌ <b>Error:</b>\n<pre>{}</pre>", escape_html(err))
}

pub fn format_success(msg: &str) -> String {
    format!("✅ <b>Success:</b>\n{}", escape_html(msg))
}
