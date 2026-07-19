#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
use base64::{engine::general_purpose, Engine as _};
use chrono::Local;
use csv::WriterBuilder;
use eframe::{egui, App};
use regex::Regex;
use rfd::FileDialog;
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::Duration,
};

const DEFAULT_NETMASK: &str = "255.255.255.0";
const DEFAULT_DNS1: &str = "1.1.1.1";
const DEFAULT_DNS2: &str = "8.8.8.8";
mod blockops_splash;
use blockops_splash::BlockOpsSplash;

const DEFAULT_LISTEN_PORT: u16 = 14235;
const PARK_START: u8 = 168;
const PARK_END: u8 = 240;

static BLOCKOPS_APP_ICON_BYTES: &[u8] = include_bytes!("../assets/blockops_app_icon.png");

fn color_app_bg() -> egui::Color32 {
    egui::Color32::from_rgb(10, 12, 15)
}

fn color_sidebar() -> egui::Color32 {
    egui::Color32::from_rgb(13, 15, 19)
}

fn color_surface() -> egui::Color32 {
    egui::Color32::from_rgb(20, 23, 28)
}

fn color_surface_high() -> egui::Color32 {
    egui::Color32::from_rgb(27, 31, 38)
}

fn color_surface_hover() -> egui::Color32 {
    egui::Color32::from_rgb(34, 39, 48)
}

fn color_border() -> egui::Color32 {
    egui::Color32::from_rgb(43, 49, 59)
}

fn color_text() -> egui::Color32 {
    egui::Color32::from_rgb(238, 241, 245)
}

fn color_text_muted() -> egui::Color32 {
    egui::Color32::from_rgb(139, 149, 163)
}

fn color_accent() -> egui::Color32 {
    egui::Color32::from_rgb(49, 143, 232)
}

fn color_accent_soft() -> egui::Color32 {
    egui::Color32::from_rgb(22, 45, 68)
}

fn color_success() -> egui::Color32 {
    egui::Color32::from_rgb(48, 197, 139)
}

fn color_warning() -> egui::Color32 {
    egui::Color32::from_rgb(235, 174, 60)
}

fn color_danger() -> egui::Color32 {
    egui::Color32::from_rgb(226, 85, 103)
}

fn install_product_fonts(ctx: &egui::Context) -> egui::FontFamily {
    let mut fonts = egui::FontDefinitions::default();
    let mut display_family = egui::FontFamily::Proportional;

    #[cfg(target_os = "windows")]
    {
        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
            fonts.font_data.insert(
                "blockops_regular".to_owned(),
                egui::FontData::from_owned(bytes),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "blockops_regular".to_owned());
            }
        }

        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisb.ttf") {
            fonts.font_data.insert(
                "blockops_semibold".to_owned(),
                egui::FontData::from_owned(bytes),
            );
            display_family = egui::FontFamily::Name("blockops_display".into());
            fonts.families.insert(
                display_family.clone(),
                vec![
                    "blockops_semibold".to_owned(),
                    "blockops_regular".to_owned(),
                ],
            );
        }

        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
            fonts.font_data.insert(
                "blockops_symbols".to_owned(),
                egui::FontData::from_owned(bytes),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.push("blockops_symbols".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&display_family) {
                if !family.iter().any(|name| name == "blockops_symbols") {
                    family.push("blockops_symbols".to_owned());
                }
            }
        }
    }

    ctx.set_fonts(fonts);
    display_family
}

fn configure_egui(ctx: &egui::Context) {
    let display_family = install_product_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(21.0, display_family.clone()),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(13.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(13.0, display_family),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(11.5, egui::FontFamily::Proportional),
    );
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(11.0, 7.0);
    style.spacing.interact_size.y = 32.0;

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = color_app_bg();
    visuals.window_fill = color_surface();
    visuals.window_stroke = egui::Stroke::new(1.0, color_border());
    visuals.window_rounding = egui::Rounding::same(8.0);
    visuals.menu_rounding = egui::Rounding::same(6.0);
    visuals.extreme_bg_color = egui::Color32::from_rgb(8, 10, 13);
    visuals.faint_bg_color = egui::Color32::from_rgb(24, 27, 33);
    visuals.override_text_color = Some(color_text());
    visuals.hyperlink_color = color_accent();
    visuals.warn_fg_color = color_warning();
    visuals.error_fg_color = color_danger();
    visuals.selection.bg_fill = color_accent();
    visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.widgets.noninteractive.bg_fill = color_surface();
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, color_border());
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(5.0);
    visuals.widgets.inactive.bg_fill = color_surface_high();
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, color_border());
    visuals.widgets.inactive.rounding = egui::Rounding::same(5.0);
    visuals.widgets.hovered.bg_fill = color_surface_hover();
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, color_accent());
    visuals.widgets.hovered.rounding = egui::Rounding::same(5.0);
    visuals.widgets.active.bg_fill = color_accent();
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, color_accent());
    visuals.widgets.active.rounding = egui::Rounding::same(5.0);
    visuals.widgets.open.bg_fill = color_surface_hover();
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, color_accent());
    visuals.widgets.open.rounding = egui::Rounding::same(5.0);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.striped = false;
    style.visuals = visuals;
    ctx.set_style(style);
}

#[cfg(target_os = "windows")]
fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn center_window_after_startup() {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetSystemMetrics, GetWindowRect, MoveWindow, SM_CXSCREEN, SM_CYSCREEN,
    };

    thread::spawn(|| {
        // Give eframe/winit a moment to create the native window.
        let title = wide_null("BlockOps Static IP Manager");

        for _ in 0..40 {
            thread::sleep(Duration::from_millis(100));

            let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
            if hwnd == std::ptr::null_mut() {
                continue;
            }

            unsafe {
                let mut rect: RECT = std::mem::zeroed();
                if GetWindowRect(hwnd, &mut rect) == 0 {
                    return;
                }

                // Roughly BTC-tool sized outer window.
                let width: i32 = 1220;
                let height: i32 = 860;

                // Center on primary monitor.
                let screen_w = GetSystemMetrics(SM_CXSCREEN);
                let screen_h = GetSystemMetrics(SM_CYSCREEN);

                let x = (screen_w - width).max(0) / 2;
                let y = (screen_h - height).max(0) / 2;

                MoveWindow(hwnd, x, y, width, height, 1);
            }

            return;
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn center_window_after_startup() {}

#[derive(Debug, Clone)]
struct MinerRow {
    line: usize,
    current_ip: String,
    target_ip: String,
    mac: String,
    status: String,
    apply_order: String,
    apply_wave: String,
    apply_type: String,
    apply_status: String,
}

#[derive(Debug, Clone)]
struct SkipRow {
    line: usize,
    skipped_ip: String,
    reason: String,
}

#[derive(Debug, Clone)]
struct ApplyStep {
    order: usize,
    wave: usize,
    row_line: usize,
    current_ip: String,
    target_ip: String,
    mac: String,
    kind: String,
    status: String,
}

#[derive(Debug, Clone)]
struct ReportPacket {
    current_ip: String,
    mac: String,
}

#[derive(Debug, Clone)]
struct WrongSubnetPopup {
    reported_ip: String,
    expected_subnet: String,
    reported_subnet: String,
    target_ip: String,
    mac: String,
}

#[derive(Debug, Clone)]
struct MinerApiDetails {
    ip: String,
    firmware: String,
    model: String,
    miner: String,
    mac: String,
    status: String,
    pool: String,
    hashrate: String,
    power: String,
    efficiency: String,
    uptime: String,
    temperature: String,
    fans: String,
    boards: String,
    updated: String,
    error: String,
}

impl MinerApiDetails {
    fn loading(ip: &str) -> Self {
        Self {
            ip: ip.to_string(),
            firmware: "Loading".to_string(),
            model: "-".to_string(),
            miner: "-".to_string(),
            mac: "-".to_string(),
            status: "Loading details...".to_string(),
            pool: "-".to_string(),
            hashrate: "-".to_string(),
            power: "-".to_string(),
            efficiency: "-".to_string(),
            uptime: "-".to_string(),
            temperature: "-".to_string(),
            fans: "-".to_string(),
            boards: "-".to_string(),
            updated: Local::now().format("%H:%M:%S").to_string(),
            error: String::new(),
        }
    }

    fn to_wire_json(&self) -> String {
        json!({
            "ip": self.ip,
            "firmware": self.firmware,
            "model": self.model,
            "miner": self.miner,
            "mac": self.mac,
            "status": self.status,
            "pool": self.pool,
            "hashrate": self.hashrate,
            "power": self.power,
            "efficiency": self.efficiency,
            "uptime": self.uptime,
            "temperature": self.temperature,
            "fans": self.fans,
            "boards": self.boards,
            "updated": self.updated,
            "error": self.error,
        })
        .to_string()
    }

    fn from_wire_json(text: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let get = |key: &str| {
            value
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string()
        };

        Some(Self {
            ip: get("ip"),
            firmware: get("firmware"),
            model: get("model"),
            miner: get("miner"),
            mac: get("mac"),
            status: get("status"),
            pool: get("pool"),
            hashrate: get("hashrate"),
            power: get("power"),
            efficiency: get("efficiency"),
            uptime: get("uptime"),
            temperature: get("temperature"),
            fans: get("fans"),
            boards: get("boards"),
            updated: get("updated"),
            error: value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotMonitorState {
    Unknown,
    VnishMiner,
    BitmainMiner,
    AuthRequired,
    WebOnline,
    SshOnly,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    RackDashboard,
    IpAssignment,
    Settings,
}

impl AppView {
    fn label(self) -> &'static str {
        match self {
            Self::RackDashboard => "Rack Dashboard",
            Self::IpAssignment => "IP Assignment",
            Self::Settings => "Settings",
        }
    }
}

impl SlotMonitorState {
    fn label(self) -> &'static str {
        match self {
            SlotMonitorState::Unknown => "Unknown",
            SlotMonitorState::VnishMiner => "VNISH miner",
            SlotMonitorState::BitmainMiner => "Bitmain miner",
            SlotMonitorState::AuthRequired => "Miner auth required",
            SlotMonitorState::WebOnline => "Web online",
            SlotMonitorState::SshOnly => "SSH only",
            SlotMonitorState::Offline => "Offline",
        }
    }

    fn wire(self) -> &'static str {
        match self {
            SlotMonitorState::Unknown => "Unknown",
            SlotMonitorState::VnishMiner => "VnishMiner",
            SlotMonitorState::BitmainMiner => "BitmainMiner",
            SlotMonitorState::AuthRequired => "AuthRequired",
            SlotMonitorState::WebOnline => "WebOnline",
            SlotMonitorState::SshOnly => "SshOnly",
            SlotMonitorState::Offline => "Offline",
        }
    }

    fn from_wire(value: &str) -> Self {
        match value {
            "VnishMiner" => SlotMonitorState::VnishMiner,
            "BitmainMiner" => SlotMonitorState::BitmainMiner,
            "AuthRequired" => SlotMonitorState::AuthRequired,
            "WebOnline" => SlotMonitorState::WebOnline,
            "SshOnly" => SlotMonitorState::SshOnly,
            "Offline" => SlotMonitorState::Offline,
            _ => SlotMonitorState::Unknown,
        }
    }

    fn is_present(self) -> bool {
        !matches!(self, SlotMonitorState::Unknown | SlotMonitorState::Offline)
    }
}

#[derive(Default)]
struct MonitorCounts {
    total: usize,
    present: usize,
    vnish: usize,
    bitmain: usize,
    auth: usize,
    web: usize,
    ssh: usize,
    offline: usize,
    unknown: usize,
}

fn normalize_mac(input: &str) -> String {
    let mut s = input
        .trim()
        .to_lowercase()
        .replace('-', ":")
        .replace('"', "");
    s.retain(|c| c.is_ascii_hexdigit() || c == ':');
    if !s.contains(':') && s.len() == 12 {
        let mut out = String::new();
        for i in (0..12).step_by(2) {
            if !out.is_empty() {
                out.push(':');
            }
            out.push_str(&s[i..i + 2]);
        }
        return out;
    }
    s
}

fn valid_ip(ip: &str) -> bool {
    ip.parse::<Ipv4Addr>().is_ok()
}

fn parse_ipv4(ip: &str) -> Option<Ipv4Addr> {
    ip.trim().parse::<Ipv4Addr>().ok()
}

fn next_ipv4(ip: Ipv4Addr) -> Option<Ipv4Addr> {
    let o = ip.octets();
    if o[3] >= 254 {
        return None;
    }
    Some(Ipv4Addr::new(o[0], o[1], o[2], o[3] + 1))
}

fn same_subnet_prefix(ip: &str) -> Option<String> {
    let o = parse_ipv4(ip)?.octets();
    Some(format!("{}.{}.{}", o[0], o[1], o[2]))
}

fn parking_ip_for_target(target_ip: &str, used: &HashSet<String>) -> Option<String> {
    let prefix = same_subnet_prefix(target_ip)?;
    for h in PARK_START..=PARK_END {
        let candidate = format!("{}.{}", prefix, h);
        if !used.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn gateway_for_target(target_ip: &str) -> String {
    let parts: Vec<&str> = target_ip.split('.').collect();
    if parts.len() == 4 {
        format!("{}.{}.{}.254", parts[0], parts[1], parts[2])
    } else {
        String::new()
    }
}

fn gateway_for_target_with_override(
    target_ip: &str,
    gateway_override: &str,
) -> Result<String, String> {
    let g = gateway_override.trim();

    if g.is_empty() || g.eq_ignore_ascii_case("auto") {
        return Ok(gateway_for_target(target_ip));
    }

    if valid_ip(g) {
        return Ok(g.to_string());
    }

    Err(format!(
        "Invalid gateway '{}'. Enter a valid gateway IP or leave it blank for auto .254.",
        g
    ))
}

fn parse_report_packet(data: &[u8], sender_ip: &str) -> ReportPacket {
    let text = String::from_utf8_lossy(data).to_string();
    let ip_re = Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap();
    let mac_re = Regex::new(r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b").unwrap();

    let mut current_ip = sender_ip.to_string();
    let mut mac = String::new();

    if let Some(m) = ip_re.find(&text) {
        let candidate = m.as_str();
        if valid_ip(candidate) {
            current_ip = candidate.to_string();
        }
    }

    if let Some(m) = mac_re.find(&text) {
        mac = normalize_mac(m.as_str());
    }

    ReportPacket { current_ip, mac }
}

fn md5_hex(input: &str) -> String {
    format!("{:x}", md5::compute(input.as_bytes()))
}

fn form_encode(input: &str) -> String {
    let mut out = String::new();

    for b in input.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
        } else if c == ' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }

    out
}

fn basic_auth_header(username: &str, password: &str) -> String {
    let raw = format!("{}:{}", username, password);
    format!("Basic {}", general_purpose::STANDARD.encode(raw.as_bytes()))
}

fn parse_digest_header(header: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let cleaned = header.trim().trim_start_matches("Digest").trim();

    for part in cleaned.split(',') {
        let mut split = part.trim().splitn(2, '=');
        let Some(key) = split.next() else {
            continue;
        };
        let Some(value) = split.next() else {
            continue;
        };

        let value = value.trim().trim_matches('"').to_string();
        map.insert(key.trim().to_lowercase(), value);
    }

    map
}

fn build_digest_auth_header(
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    www_auth: &str,
) -> Result<String, String> {
    let fields = parse_digest_header(www_auth);

    let realm = fields
        .get("realm")
        .ok_or_else(|| "digest missing realm".to_string())?;
    let nonce = fields
        .get("nonce")
        .ok_or_else(|| "digest missing nonce".to_string())?;
    let qop_raw = fields.get("qop").cloned().unwrap_or_default();
    let opaque = fields.get("opaque").cloned();

    let nc = "00000001";
    let cnonce = md5_hex(&format!(
        "{}:{}:{}",
        username,
        uri,
        Local::now().timestamp_millis()
    ));

    let ha1 = md5_hex(&format!("{}:{}:{}", username, realm, password));
    let ha2 = md5_hex(&format!("{}:{}", method, uri));

    let use_qop_auth = qop_raw
        .split(',')
        .map(|s| s.trim().trim_matches('"'))
        .any(|s| s.eq_ignore_ascii_case("auth"));

    let response = if use_qop_auth {
        md5_hex(&format!(
            "{}:{}:{}:{}:{}:{}",
            ha1, nonce, nc, cnonce, "auth", ha2
        ))
    } else {
        md5_hex(&format!("{}:{}:{}", ha1, nonce, ha2))
    };

    let mut header = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\"",
        username, realm, nonce, uri, response
    );

    if use_qop_auth {
        header.push_str(&format!(", qop=auth, nc={}, cnonce=\"{}\"", nc, cnonce));
    }

    if let Some(opaque) = opaque {
        header.push_str(&format!(", opaque=\"{}\"", opaque));
    }

    Ok(header)
}

fn stock_hiveon_form_body(
    target_ip: &str,
    netmask: &str,
    dns1: &str,
    gateway_override: &str,
) -> Result<String, String> {
    let hostname = format!("miner-{}", target_ip.replace('.', "-"));
    let gateway = gateway_for_target_with_override(target_ip, gateway_override)?;

    Ok(format!(
        "_ant_conf_nettype=Static&_ant_conf_hostname={}&_ant_conf_ipaddress={}&_ant_conf_netmask={}&_ant_conf_gateway={}&_ant_conf_dnsservers={}",
        form_encode(&hostname),
        form_encode(target_ip),
        form_encode(netmask),
        form_encode(&gateway),
        form_encode(dns1),
    ))
}

fn bitmain_stock_json_body(
    target_ip: &str,
    netmask: &str,
    dns1: &str,
    gateway_override: &str,
) -> Result<String, String> {
    let hostname = format!("miner-{}", target_ip.replace('.', "-"));
    let gateway = gateway_for_target_with_override(target_ip, gateway_override)?;

    Ok(serde_json::json!({
        "ipHost": hostname,
        "ipPro": 2,
        "ipAddress": target_ip,
        "ipSub": netmask,
        "ipGateway": gateway,
        "ipDns": dns1
    })
    .to_string())
}

fn is_network_change_disconnect(error_text: &str) -> bool {
    let e = error_text.to_lowercase();
    e.contains("forcibly closed")
        || e.contains("connection reset")
        || e.contains("connection closed")
        || e.contains("connection aborted")
        || e.contains("unexpected eof")
        || e.contains("timed out")
        || e.contains("timeout")
}

fn response_looks_successful(body: &str) -> Result<(), String> {
    let b = body.trim();
    let lower = b.to_lowercase();

    if lower.contains("\"stats\":\"error\"")
        || lower.contains("\"stats\": \"error\"")
        || lower.contains("\"status\":\"error\"")
        || lower.contains("\"status\": \"error\"")
    {
        return Err(format!("endpoint returned error: {}", b));
    }

    // Legacy shell CGI returns "ok". Newer Bitmain API returns JSON:
    // {"stats":"success","code":"N000","msg":"OK!"}
    if b.is_empty()
        || lower == "ok"
        || lower.contains("\"stats\":\"success\"")
        || lower.contains("\"stats\": \"success\"")
        || lower.contains("\"msg\":\"ok")
        || lower.contains("\"msg\": \"ok")
    {
        return Ok(());
    }

    // Some firmware returns HTML/reload or no useful JSON after accepting the change.
    // Treat HTTP 200 with non-error body as accepted, but preserve body in logs through caller if needed.
    Ok(())
}

fn post_cgi_with_auth(
    current_ip: &str,
    uri: &str,
    username: &str,
    password: &str,
    content_type: &str,
    body: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    let url = format!("http://{}{}", current_ip, uri);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .build();

    let first = agent
        .post(&url)
        .set("Content-Type", content_type)
        .send_string(body);

    match first {
        Ok(response) => {
            let text = response.into_string().unwrap_or_default();
            response_looks_successful(&text)
        }
        Err(ureq::Error::Status(401, response)) => {
            let www_auth = response
                .header("WWW-Authenticate")
                .unwrap_or("")
                .to_string();

            if www_auth.to_lowercase().contains("digest") {
                let auth_header =
                    build_digest_auth_header(username, password, "POST", uri, &www_auth)?;

                let second = agent
                    .post(&url)
                    .set("Content-Type", content_type)
                    .set("Authorization", &auth_header)
                    .send_string(body);

                match second {
                    Ok(response) => {
                        let text = response.into_string().unwrap_or_default();
                        response_looks_successful(&text)
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if is_network_change_disconnect(&msg) {
                            Ok(())
                        } else {
                            Err(format!("digest post failed: {}", msg))
                        }
                    }
                }
            } else {
                let basic_auth = basic_auth_header(username, password);

                let basic = agent
                    .post(&url)
                    .set("Content-Type", content_type)
                    .set("Authorization", &basic_auth)
                    .send_string(body);

                match basic {
                    Ok(response) => {
                        let text = response.into_string().unwrap_or_default();
                        response_looks_successful(&text)
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if is_network_change_disconnect(&msg) {
                            Ok(())
                        } else {
                            Err(format!("basic post failed: {}", msg))
                        }
                    }
                }
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if is_network_change_disconnect(&msg) {
                Ok(())
            } else {
                Err(format!("post failed: {}", msg))
            }
        }
    }
}

fn apply_stock_hiveon_static(
    current_ip: &str,
    target_ip: &str,
    username: &str,
    password: &str,
    netmask: &str,
    dns1: &str,
    gateway_override: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    let uri = "/cgi-bin/set_network_conf.cgi";
    // Official newer Bitmain docs use JSON fields on this same endpoint.
    let json_body = bitmain_stock_json_body(target_ip, netmask, dns1, gateway_override)?;

    let json_result = post_cgi_with_auth(
        current_ip,
        uri,
        username,
        password,
        "application/json",
        &json_body,
        timeout_secs.max(15),
    );

    if json_result.is_ok() {
        return Ok(());
    }

    // Older Bitmain/Hiveon-style CGI scripts use form fields beginning with _ant_conf_*.
    // Keep this fallback so the same mode works across older stock, Hiveon stock-like, and newer stock.
    let form_body = stock_hiveon_form_body(target_ip, netmask, dns1, gateway_override)?;
    let form_result = post_cgi_with_auth(
        current_ip,
        uri,
        username,
        password,
        "application/x-www-form-urlencoded",
        &form_body,
        timeout_secs.max(15),
    );

    match form_result {
        Ok(_) => Ok(()),
        Err(form_err) => Err(format!(
            "Bitmain JSON failed; legacy form failed too. JSON error: {}; form error: {}",
            json_result.err().unwrap_or_else(|| "unknown".to_string()),
            form_err
        )),
    }
}

fn apply_static_by_mode(
    firmware_mode: &str,
    current_ip: &str,
    mac: &str,
    target_ip: &str,
    stock_user: &str,
    stock_password: &str,
    vnish_password: &str,
    netmask: &str,
    dns1: &str,
    dns2: &str,
    gateway_override: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    let mode = firmware_mode.to_lowercase();

    if mode == "vnish_api" {
        return apply_vnish_static(
            current_ip,
            mac,
            target_ip,
            vnish_password,
            netmask,
            dns1,
            dns2,
            gateway_override,
            timeout_secs,
        );
    }

    if mode == "stock_hiveon_cgi" {
        return apply_stock_hiveon_static(
            current_ip,
            target_ip,
            stock_user,
            stock_password,
            netmask,
            dns1,
            gateway_override,
            timeout_secs.max(15),
        );
    }

    // Auto mode:
    // 1. Try VNISH token/API first. VNISH usually fails fast on stock firmware.
    // 2. If VNISH does not accept it, try Bitmain Stock/Hiveon CGI.
    //
    // Keep the VNISH probe short so a mixed rack does not waste tons of time on stock units.
    let vnish_timeout = timeout_secs.min(5).max(3);
    let vnish_result = apply_vnish_static(
        current_ip,
        mac,
        target_ip,
        vnish_password,
        netmask,
        dns1,
        dns2,
        gateway_override,
        vnish_timeout,
    );

    if vnish_result.is_ok() {
        return Ok(());
    }

    let vnish_error = vnish_result
        .err()
        .unwrap_or_else(|| "unknown VNISH error".to_string());

    let stock_result = apply_stock_hiveon_static(
        current_ip,
        target_ip,
        stock_user,
        stock_password,
        netmask,
        dns1,
        gateway_override,
        timeout_secs.max(15),
    );

    match stock_result {
        Ok(_) => Ok(()),
        Err(stock_error) => Err(format!(
            "AUTO failed. VNISH: {}; Stock/Hiveon: {}",
            vnish_error, stock_error
        )),
    }
}

fn get_token(ip: &str, password: &str, timeout_secs: u64) -> Result<String, String> {
    let url = format!("http://{}/api/v1/unlock", ip);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .build();

    let response = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(json!({ "pw": password }))
        .map_err(|e| format!("unlock failed: {}", e))?;

    let body = response.into_string().map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;

    value["token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "unlock response had no token".to_string())
}

fn apply_vnish_static(
    current_ip: &str,
    mac: &str,
    target_ip: &str,
    password: &str,
    netmask: &str,
    dns1: &str,
    dns2: &str,
    gateway_override: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    let token = get_token(current_ip, password, timeout_secs)?;
    let url = format!("http://{}/api/v1/settings", current_ip);

    let hostname = if mac.is_empty() {
        format!("miner-{}", target_ip.replace('.', "-"))
    } else {
        format!("miner-{}", normalize_mac(mac).replace(':', "-"))
    };

    let gateway = gateway_for_target_with_override(target_ip, gateway_override)?;

    let payload = json!({
        "network": {
            "hostname": hostname,
            "dhcp": false,
            "ipaddress": target_ip,
            "netmask": netmask,
            "gateway": gateway,
            "dnsservers": [dns1, dns2],
            "enable_network_check": false
        }
    });

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .build();

    agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set("Authorization", &format!("Bearer {}", token))
        .send_json(payload)
        .map_err(|e| format!("settings failed: {}", e))?;

    Ok(())
}

fn http_status_code(current_ip: &str, path: &str, timeout_secs: u64) -> Option<u16> {
    let url = format!("http://{}{}", current_ip, path);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs.max(2).min(10)))
        .build();

    match agent.get(&url).call() {
        Ok(resp) => Some(resp.status()),
        Err(ureq::Error::Status(code, _)) => Some(code),
        Err(_) => None,
    }
}

fn tcp_port_open(ip: &str, port: u16, timeout_ms: u64) -> bool {
    let Ok(parsed_ip) = ip.parse::<IpAddr>() else {
        return false;
    };

    let addr = SocketAddr::new(parsed_ip, port);
    TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).is_ok()
}

fn http_status_code_ms(current_ip: &str, path: &str, timeout_ms: u64) -> Option<u16> {
    let url = format!("http://{}{}", current_ip, path);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(timeout_ms.max(250).min(5000)))
        .build();

    match agent.get(&url).call() {
        Ok(response) => Some(response.status()),
        Err(ureq::Error::Status(code, _)) => Some(code),
        Err(_) => None,
    }
}

fn http_get_text_ms(current_ip: &str, path: &str, timeout_ms: u64) -> Result<(u16, String), u16> {
    let url = format!("http://{}{}", current_ip, path);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(timeout_ms.max(250).min(5000)))
        .build();

    match agent.get(&url).call() {
        Ok(response) => {
            let code = response.status();
            let body = response.into_string().unwrap_or_default();
            Ok((code, body))
        }
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            Ok((code, body))
        }
        Err(_) => Err(0),
    }
}

fn body_looks_like_vnish_miner(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("miner")
        || lower.contains("firmware")
        || lower.contains("model")
        || lower.contains("chains")
        || lower.contains("hashrate")
        || lower.contains("anthill")
        || lower.contains("xminer")
        || lower.contains("neopool")
}

fn probe_vnish_api(ip: &str, stop: Option<&AtomicBool>) -> Option<SlotMonitorState> {
    for path in [
        "/api/v1/status",
        "/api/v1/info",
        "/api/v1/model",
        "/api/v1/summary",
    ] {
        if stop.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return None;
        }

        match http_get_text_ms(ip, path, 1200) {
            Ok((401 | 403, _)) => return Some(SlotMonitorState::AuthRequired),
            Ok((200, body)) if body_looks_like_vnish_miner(&body) => {
                return Some(SlotMonitorState::VnishMiner);
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }

    None
}

fn probe_bitmain_cgi(ip: &str, stop: Option<&AtomicBool>) -> Option<SlotMonitorState> {
    for path in [
        "/cgi-bin/get_network_info.cgi",
        "/cgi-bin/get_system_info.cgi",
        "/cgi-bin/summary.cgi",
        "/cgi-bin/minerStatus.cgi",
        "/cgi-bin/set_network_conf.cgi",
    ] {
        if stop.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return None;
        }

        match http_status_code_ms(ip, path, 1200) {
            Some(200) | Some(401) | Some(403) => return Some(SlotMonitorState::BitmainMiner),
            _ => continue,
        }
    }

    None
}

fn discover_monitor_state_with_stop(ip: &str, stop: Option<&AtomicBool>) -> SlotMonitorState {
    let http_open = tcp_port_open(ip, 80, 500);
    if stop.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return SlotMonitorState::Unknown;
    }
    let ssh_open = tcp_port_open(ip, 22, 500);

    if !http_open {
        return if ssh_open {
            SlotMonitorState::SshOnly
        } else {
            SlotMonitorState::Offline
        };
    }

    if let Some(state) = probe_vnish_api(ip, stop) {
        return state;
    }

    if let Some(state) = probe_bitmain_cgi(ip, stop) {
        return state;
    }

    SlotMonitorState::WebOnline
}

fn discover_monitor_state(ip: &str) -> SlotMonitorState {
    discover_monitor_state_with_stop(ip, None)
}

fn json_find_string(value: &serde_json::Value, names: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if names.iter().any(|name| key.eq_ignore_ascii_case(name)) {
                    if let Some(s) = val.as_str() {
                        if !s.trim().is_empty() {
                            return Some(s.trim().to_string());
                        }
                    } else if val.is_number() || val.is_boolean() {
                        return Some(val.to_string());
                    }
                }
            }

            for val in map.values() {
                if let Some(found) = json_find_string(val, names) {
                    return Some(found);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for val in items {
                if let Some(found) = json_find_string(val, names) {
                    return Some(found);
                }
            }
        }
        _ => {}
    }

    None
}

fn json_find_number(value: &serde_json::Value, names: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if names.iter().any(|name| key.eq_ignore_ascii_case(name)) {
                    if let Some(n) = val.as_f64() {
                        return Some(n);
                    }
                    if let Some(s) = val.as_str() {
                        if let Ok(n) = s.replace(',', "").parse::<f64>() {
                            return Some(n);
                        }
                    }
                }
            }

            for val in map.values() {
                if let Some(found) = json_find_number(val, names) {
                    return Some(found);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for val in items {
                if let Some(found) = json_find_number(val, names) {
                    return Some(found);
                }
            }
        }
        _ => {}
    }

    None
}

fn first_non_dash(values: &[Option<String>]) -> String {
    values
        .iter()
        .flatten()
        .find(|s| !s.trim().is_empty() && s.trim() != "-")
        .cloned()
        .unwrap_or_else(|| "-".to_string())
}

fn format_uptime(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

fn format_hashrate(value: f64) -> String {
    if value <= 0.0 {
        return "-".to_string();
    }

    if value > 1_000_000.0 {
        format!("{:.2} TH/s", value / 1_000_000.0)
    } else if value > 1_000.0 {
        format!("{:.2} TH/s", value / 1_000.0)
    } else {
        format!("{:.2} TH/s", value)
    }
}

fn details_from_json(
    ip: &str,
    values: &[serde_json::Value],
    fallback_state: SlotMonitorState,
) -> MinerApiDetails {
    let pick_string = |names: &[&str]| -> String {
        first_non_dash(
            &values
                .iter()
                .map(|value| json_find_string(value, names))
                .collect::<Vec<_>>(),
        )
    };

    let pick_number = |names: &[&str]| -> Option<f64> {
        values
            .iter()
            .find_map(|value| json_find_number(value, names))
    };

    let model = pick_string(&["model", "model_id", "platform", "miner_type", "type"]);
    let miner = pick_string(&["miner", "miner_name", "device", "hostname", "ipHost"]);
    let firmware = pick_string(&["firmware", "fw_version", "version", "build", "system"]);
    let mac = pick_string(&["mac", "macaddr", "mac_address", "ethaddr"]);
    let status = pick_string(&["status", "miner_status", "state", "miner_state"]);
    let pool = pick_string(&["pool", "pool_url", "url", "active_pool"]);

    let hashrate = pick_number(&[
        "hashrate",
        "hashrate_5s",
        "hashrate_avg",
        "GHS 5s",
        "GHS av",
        "rate_5s",
    ])
    .map(format_hashrate)
    .unwrap_or_else(|| pick_string(&["hashrate", "GHS 5s", "GHS av"]));

    let power = pick_number(&["power", "power_consumption", "watt", "watts", "consumption"])
        .map(|n| format!("{:.0} W", n))
        .unwrap_or_else(|| pick_string(&["power", "power_consumption", "consumption"]));

    let efficiency = pick_number(&["efficiency", "j_th", "j/ths", "w_th"])
        .map(|n| format!("{:.1} J/TH", n))
        .unwrap_or_else(|| pick_string(&["efficiency", "j_th", "j/ths", "w_th"]));

    let uptime = pick_number(&["uptime", "elapsed", "Elapsed"])
        .map(format_uptime)
        .unwrap_or_else(|| pick_string(&["uptime", "elapsed", "Elapsed"]));

    let temperature = pick_number(&["temperature", "temp", "temp_max", "chip_temp", "pcb_temp"])
        .map(|n| format!("{:.0} C", n))
        .unwrap_or_else(|| pick_string(&["temperature", "temp", "temp_max"]));

    let fans = pick_string(&["fans", "fan", "fan_num", "fan_speed", "fan1", "fan2"]);
    let boards = pick_string(&["boards", "chains", "chain_num", "hashboard_count"]);

    MinerApiDetails {
        ip: ip.to_string(),
        firmware: if firmware == "-" {
            fallback_state.label().to_string()
        } else {
            firmware
        },
        model,
        miner,
        mac,
        status: if status == "-" {
            fallback_state.label().to_string()
        } else {
            status
        },
        pool,
        hashrate,
        power,
        efficiency,
        uptime,
        temperature,
        fans,
        boards,
        updated: Local::now().format("%H:%M:%S").to_string(),
        error: String::new(),
    }
}

fn fetch_miner_api_details(ip: &str) -> MinerApiDetails {
    let state = discover_monitor_state(ip);
    let mut values = Vec::new();
    let mut errors = Vec::new();

    for path in [
        "/api/v1/status",
        "/api/v1/info",
        "/api/v1/model",
        "/api/v1/summary",
        "/api/v1/chains",
    ] {
        match http_get_text_ms(ip, path, 1500) {
            Ok((200, body)) => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
                    values.push(value);
                }
            }
            Ok((401 | 403, _)) => {}
            Ok((code, _)) => errors.push(format!("{} HTTP {}", path, code)),
            Err(_) => {}
        }
    }

    for path in [
        "/cgi-bin/get_network_info.cgi",
        "/cgi-bin/get_system_info.cgi",
        "/cgi-bin/summary.cgi",
        "/cgi-bin/minerStatus.cgi",
    ] {
        match http_get_text_ms(ip, path, 1500) {
            Ok((200, body)) => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
                    values.push(value);
                }
            }
            Ok((401 | 403, _)) => {}
            Ok((code, _)) => errors.push(format!("{} HTTP {}", path, code)),
            Err(_) => {}
        }
    }

    if values.is_empty() {
        let mut details = MinerApiDetails::loading(ip);
        details.firmware = state.label().to_string();
        details.status = state.label().to_string();
        details.updated = Local::now().format("%H:%M:%S").to_string();
        details.error = if errors.is_empty() {
            "No readable API detail response.".to_string()
        } else {
            errors.join("; ")
        };
        return details;
    }

    details_from_json(ip, &values, state)
}

fn precheck_vnish(
    current_ip: &str,
    vnish_password: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    match get_token(current_ip, vnish_password, timeout_secs.max(3).min(8)) {
        Ok(_) => Ok("VNISH API OK".to_string()),
        Err(e) => {
            if vnish_password != "root" {
                if get_token(current_ip, "root", timeout_secs.max(3).min(8)).is_ok() {
                    return Ok("VNISH API OK with root".to_string());
                }
            }
            Err(format!("VNISH auth/API failed: {}", e))
        }
    }
}

fn precheck_stock_hiveon(
    current_ip: &str,
    _username: &str,
    _password: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    // Keep this as a non-mutating reachability/API-shape check.
    // We avoid using curl helpers here so the precheck compiles cleanly across builds.
    // Many stock/Hiveon endpoints will return 401/403 without auth, which still proves
    // the CGI endpoint exists and the firmware is reachable.
    let endpoints = [
        "/cgi-bin/get_network_info.cgi",
        "/cgi-bin/get_system_info.cgi",
        "/cgi-bin/summary.cgi",
        "/cgi-bin/minerStatus.cgi",
        "/cgi-bin/set_network_conf.cgi",
    ];

    for ep in endpoints {
        if let Some(code) = http_status_code(current_ip, ep, timeout_secs.max(3).min(8)) {
            if matches!(code, 200..=299) {
                return Ok(format!("Stock/Hiveon CGI reachable ({}) HTTP {}", ep, code));
            }

            if matches!(code, 401 | 403 | 405) {
                return Ok(format!("Stock/Hiveon CGI detected ({}) HTTP {}", ep, code));
            }
        }
    }

    Err("Stock/Hiveon CGI not confirmed".to_string())
}

fn precheck_any_firmware(
    current_ip: &str,
    stock_user: &str,
    stock_password: &str,
    vnish_password: &str,
    timeout_secs: u64,
) -> String {
    if !valid_ip(current_ip) {
        return "PRECHECK skipped: invalid/unfilled current IP".to_string();
    }

    let root_status = http_status_code(current_ip, "/", timeout_secs.max(2).min(5));
    if root_status.is_none() {
        return "PRECHECK FAIL: no web response".to_string();
    }

    match precheck_vnish(current_ip, vnish_password, timeout_secs) {
        Ok(msg) => format!("PRECHECK OK: {}", msg),
        Err(vnish_err) => {
            match precheck_stock_hiveon(current_ip, stock_user, stock_password, timeout_secs) {
                Ok(msg) => format!("PRECHECK OK: {}", msg),
                Err(stock_err) => format!(
                    "PRECHECK WARN: web HTTP {:?}, but API not confirmed. VNISH: {}; Stock/Hiveon: {}",
                    root_status, vnish_err, stock_err
                ),
            }
        }
    }
}

fn spawn_udp_listener(
    port: u16,
    tx: Sender<ReportPacket>,
    status_tx: Sender<String>,
    stop_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let socket = match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(s) => s,
            Err(e) => {
                let _ = status_tx.send(format!("Could not bind UDP {}: {}", port, e));
                return;
            }
        };

        let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = status_tx.send(format!("Listening for IP reports on UDP {}", port));
        let mut buf = [0u8; 4096];

        while !stop_flag.load(Ordering::Relaxed) {
            match socket.recv_from(&mut buf) {
                Ok((len, addr)) => {
                    let report = parse_report_packet(&buf[..len], &addr.ip().to_string());
                    let _ = tx.send(report);
                }
                Err(_) => continue,
            }
        }

        let _ = status_tx.send("IP report listener stopped.".to_string());
    });
}

struct BlockOpsApp {
    active_view: AppView,
    rows: Vec<MinerRow>,
    skips: Vec<SkipRow>,
    apply_steps: Vec<ApplyStep>,

    assigned_current_ips: HashSet<String>,
    assigned_macs: HashSet<String>,
    assigned_target_ips: HashSet<String>,

    start_ip_input: String,
    next_target_ip: Option<Ipv4Addr>,
    skip_reason_input: String,
    rack_one_slot_one_ip: String,
    rack_count: usize,
    rack_size: usize,
    edit_rack_map: bool,
    selected_detail_slot: Option<(usize, usize)>,
    selected_rack_slot: Option<(usize, usize)>,
    armed_target_ip: Option<String>,
    auto_apply_armed_reports: bool,
    apply_running: bool,
    apply_queued: bool,
    monitor_running: bool,
    monitor_live: bool,
    monitor_stop: Option<Arc<AtomicBool>>,
    monitor_interval_secs: u64,
    monitor_rack_input: String,
    monitor_results: HashMap<String, SlotMonitorState>,
    monitor_failure_streaks: HashMap<String, u8>,
    monitor_last_checked: HashMap<String, String>,
    monitor_last_seen: HashMap<String, String>,
    miner_details: HashMap<String, MinerApiDetails>,
    details_loading: HashSet<String>,

    vnish_password: String,
    stock_user: String,
    stock_password: String,
    netmask: String,
    dns1: String,
    dns2: String,
    gateway_override: String,
    timeout_secs: u64,
    apply_delay_secs: u64,
    parallel_jobs: usize,
    listen_port: u16,

    reject_wrong_subnet_reports: bool,
    wrong_subnet_popup: Option<WrongSubnetPopup>,

    report_rx: Receiver<ReportPacket>,
    report_tx: Sender<ReportPacket>,
    status_rx: Receiver<String>,
    status_tx: Sender<String>,

    listener_started: bool,
    listener_stop: Option<Arc<AtomicBool>>,
    status: String,
    log_path: String,
    apply_results_path: String,

    splash_done: bool,
    blockops_splash: BlockOpsSplash,

    selected_line: Option<usize>,
    redo_target_ip: Option<String>,
    redo_row_line: Option<usize>,
    auto_scroll_miners: bool,
    scroll_to_bottom_next: bool,
}

impl Default for BlockOpsApp {
    fn default() -> Self {
        let (report_tx, report_rx) = mpsc::channel();
        let (status_tx, status_rx) = mpsc::channel();

        Self {
            active_view: AppView::RackDashboard,
            rows: Vec::new(),
            skips: Vec::new(),
            apply_steps: Vec::new(),

            assigned_current_ips: HashSet::new(),
            assigned_macs: HashSet::new(),
            assigned_target_ips: HashSet::new(),

            start_ip_input: "".to_string(),
            next_target_ip: None,
            skip_reason_input: "No report".to_string(),
            rack_one_slot_one_ip: "10.4.1.1".to_string(),
            rack_count: 19,
            rack_size: 168,
            edit_rack_map: false,
            selected_detail_slot: None,
            selected_rack_slot: None,
            armed_target_ip: None,
            auto_apply_armed_reports: true,
            apply_running: false,
            apply_queued: false,
            monitor_running: false,
            monitor_live: false,
            monitor_stop: None,
            monitor_interval_secs: 30,
            monitor_rack_input: "1".to_string(),
            monitor_results: HashMap::new(),
            monitor_failure_streaks: HashMap::new(),
            monitor_last_checked: HashMap::new(),
            monitor_last_seen: HashMap::new(),
            miner_details: HashMap::new(),
            details_loading: HashSet::new(),

            vnish_password: "admin".to_string(),
            stock_user: "root".to_string(),
            stock_password: "root".to_string(),
            netmask: DEFAULT_NETMASK.to_string(),
            dns1: DEFAULT_DNS1.to_string(),
            dns2: DEFAULT_DNS2.to_string(),
            gateway_override: String::new(),
            timeout_secs: 8,
            apply_delay_secs: 2,
            parallel_jobs: 12,
            listen_port: DEFAULT_LISTEN_PORT,

            reject_wrong_subnet_reports: true,
            wrong_subnet_popup: None,

            report_rx,
            report_tx,
            status_rx,
            status_tx,

            listener_started: false,
            listener_stop: None,
            status: "Ready. Choose Rack Dashboard or IP Assignment to begin.".to_string(),
            log_path: format!(
                "blockops_static_ip_{}.log",
                Local::now().format("%Y%m%d_%H%M%S")
            ),
            apply_results_path: format!(
                "blockops_apply_results_{}.csv",
                Local::now().format("%Y%m%d_%H%M%S")
            ),

            splash_done: false,
            blockops_splash: BlockOpsSplash::default(),

            selected_line: None,
            redo_target_ip: None,
            redo_row_line: None,
            auto_scroll_miners: true,
            scroll_to_bottom_next: false,
        }
    }
}

impl BlockOpsApp {
    fn log(&self, msg: &str) {
        if let Ok(mut f) = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.log_path)
        {
            let _ = writeln!(f, "[{}] {}", Local::now().format("%H:%M:%S"), msg);
        }
    }

    fn set_start_ip(&mut self) {
        match parse_ipv4(&self.start_ip_input) {
            Some(ip) => {
                self.next_target_ip = Some(ip);
                self.status = format!("Next target IP set to {}", ip);
                self.log(&self.status);
            }
            None => self.status = "Invalid start IP.".to_string(),
        }
    }

    fn rack_slot_target_ip(&self, rack: usize, slot: usize) -> Option<String> {
        if rack == 0 || slot == 0 || slot > self.rack_size {
            return None;
        }

        let base = parse_ipv4(&self.rack_one_slot_one_ip)?;
        let mut o = base.octets();
        let rack_offset = rack.checked_sub(1)?;
        let third = (o[2] as usize).checked_add(rack_offset)?;

        if third > 254 || slot > 254 {
            return None;
        }

        o[2] = third as u8;
        o[3] = slot as u8;
        Some(Ipv4Addr::new(o[0], o[1], o[2], o[3]).to_string())
    }

    fn monitor_counts(&self) -> MonitorCounts {
        let mut counts = MonitorCounts::default();

        for rack in 1..=self.rack_count {
            for slot in 1..=self.rack_size {
                let Some(ip) = self.rack_slot_target_ip(rack, slot) else {
                    continue;
                };
                counts.total += 1;

                let state = self
                    .monitor_results
                    .get(&ip)
                    .copied()
                    .unwrap_or(SlotMonitorState::Unknown);

                if state.is_present() {
                    counts.present += 1;
                }

                match state {
                    SlotMonitorState::VnishMiner => counts.vnish += 1,
                    SlotMonitorState::BitmainMiner => counts.bitmain += 1,
                    SlotMonitorState::AuthRequired => counts.auth += 1,
                    SlotMonitorState::WebOnline => counts.web += 1,
                    SlotMonitorState::SshOnly => counts.ssh += 1,
                    SlotMonitorState::Offline => counts.offline += 1,
                    SlotMonitorState::Unknown => counts.unknown += 1,
                }
            }
        }

        counts
    }

    fn arm_rack_slot(&mut self, rack: usize, slot: usize) {
        let Some(target_ip) = self.rack_slot_target_ip(rack, slot) else {
            self.status =
                "Rack map IP rule is invalid. Check Rack 1 Slot 1 IP and rack size.".to_string();
            return;
        };

        self.selected_rack_slot = Some((rack, slot));
        self.selected_detail_slot = Some((rack, slot));
        self.armed_target_ip = Some(target_ip.clone());
        self.status = format!(
            "Armed Rack {} Slot {} for {}. Press IP Report on that physical miner.",
            rack, slot, target_ip
        );
        self.log(&self.status);
    }

    fn slot_assignment(&self, target_ip: &str) -> Option<&MinerRow> {
        self.rows.iter().find(|r| r.target_ip == target_ip)
    }

    fn accept_report_for_armed_slot(&mut self, current_ip: String, mac: String) -> bool {
        let Some(target_ip) = self.armed_target_ip.clone() else {
            return false;
        };

        let (rack, slot) = self.selected_rack_slot.unwrap_or((0, 0));

        if self.block_wrong_subnet_report_if_needed(&current_ip, &mac, &target_ip) {
            return true;
        }

        if self
            .rows
            .iter()
            .any(|r| r.target_ip != target_ip && r.current_ip == current_ip)
        {
            self.status = format!(
                "Armed report blocked: current IP {} is already used on another target.",
                current_ip
            );
            return true;
        }

        if !mac.is_empty()
            && self
                .rows
                .iter()
                .any(|r| r.target_ip != target_ip && r.mac == mac)
        {
            self.status = format!(
                "Armed report blocked: MAC {} is already used on another target.",
                mac
            );
            return true;
        }

        if let Some(pos) = self.rows.iter().position(|r| r.target_ip == target_ip) {
            let old_current = self.rows[pos].current_ip.clone();
            let old_mac = self.rows[pos].mac.clone();

            self.rows[pos].current_ip = current_ip.clone();
            self.rows[pos].mac = mac.clone();
            self.rows[pos].status = format!(
                "Rack {} Slot {} report replaced {} / {}",
                rack, slot, old_current, old_mac
            );
        } else {
            self.rows.push(MinerRow {
                line: self.rows.len() + 1,
                current_ip: current_ip.clone(),
                target_ip: target_ip.clone(),
                mac: mac.clone(),
                status: format!("Rack {} Slot {} mapped", rack, slot),
                apply_order: "".to_string(),
                apply_wave: "".to_string(),
                apply_type: "".to_string(),
                apply_status: "".to_string(),
            });
        }

        self.rebuild_assignment_sets();
        if !self.apply_running {
            self.apply_steps.clear();
        }
        self.scroll_to_bottom_next = true;
        self.armed_target_ip = None;

        self.status = format!(
            "Captured Rack {} Slot {}: {} -> {}. {}",
            rack,
            slot,
            current_ip,
            target_ip,
            if self.auto_apply_armed_reports {
                "Auto-apply started."
            } else {
                "Ready to apply."
            }
        );
        self.log(&self.status);

        if self.auto_apply_armed_reports {
            self.apply_safe_order();
        }

        true
    }

    fn start_monitor_scan(&mut self, continuous: bool, rack_only: Option<usize>) {
        if self.monitor_running {
            self.status = "Monitor scan is already running.".to_string();
            return;
        }

        let mut targets = Vec::new();
        let rack_start = rack_only.unwrap_or(1);
        let rack_end = rack_only.unwrap_or(self.rack_count);

        if rack_start == 0 || rack_end > self.rack_count {
            self.status = "Select a rack within the configured rack count.".to_string();
            return;
        }

        for rack in rack_start..=rack_end {
            for slot in 1..=self.rack_size {
                if let Some(ip) = self.rack_slot_target_ip(rack, slot) {
                    targets.push(ip);
                }
            }
        }

        if targets.is_empty() {
            self.status = "Monitor scan could not start. Check rack map IP settings.".to_string();
            return;
        }

        let parallel_jobs = self.parallel_jobs.max(1).min(128);
        let tx = self.status_tx.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let interval_secs = self.monitor_interval_secs.max(5);
        self.monitor_running = true;
        self.monitor_live = continuous;
        self.monitor_stop = Some(stop);
        self.status = format!(
            "{} started for {} rack slots with {} parallel checks{}.",
            if continuous {
                "Live monitor"
            } else {
                "Monitor scan"
            },
            targets.len(),
            parallel_jobs,
            rack_only
                .map(|rack| format!(" on Rack {}", rack))
                .unwrap_or_default()
        );

        thread::spawn(move || {
            loop {
                for chunk in targets.chunks(parallel_jobs) {
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }

                    let mut handles = Vec::new();

                    for ip in chunk.iter().cloned() {
                        let tx_slot = tx.clone();
                        let stop_slot = stop_thread.clone();
                        handles.push(thread::spawn(move || {
                            let state =
                                discover_monitor_state_with_stop(&ip, Some(stop_slot.as_ref()));
                            if !stop_slot.load(Ordering::Relaxed) {
                                let _ =
                                    tx_slot.send(format!("MONITOR_RESULT|{}|{}", ip, state.wire()));
                            }
                        }));
                    }

                    for handle in handles {
                        let _ = handle.join();
                    }
                }

                if !continuous || stop_thread.load(Ordering::Relaxed) {
                    break;
                }

                let _ = tx.send("MONITOR_CYCLE_DONE".to_string());

                for _ in 0..interval_secs {
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
            let _ = tx.send("MONITOR_DONE".to_string());
        });
    }

    fn stop_monitor_scan(&mut self) {
        if let Some(flag) = &self.monitor_stop {
            flag.store(true, Ordering::Relaxed);
            self.status = "Stopping live monitor...".to_string();
        }
    }

    fn select_rack_slot_details(&mut self, rack: usize, slot: usize) {
        self.selected_detail_slot = Some((rack, slot));
        self.selected_rack_slot = Some((rack, slot));
        let target = self
            .rack_slot_target_ip(rack, slot)
            .unwrap_or_else(|| "invalid".to_string());
        self.status = format!("Viewing Rack {} Slot {} ({})", rack, slot, target);
        if valid_ip(&target) {
            self.request_miner_details(target);
        }
    }

    fn request_miner_details(&mut self, ip: String) {
        if self.details_loading.contains(&ip) {
            return;
        }

        self.details_loading.insert(ip.clone());
        self.miner_details
            .entry(ip.clone())
            .or_insert_with(|| MinerApiDetails::loading(&ip));

        let tx = self.status_tx.clone();
        thread::spawn(move || {
            let details = fetch_miner_api_details(&ip);
            let _ = tx.send(format!("DETAIL_RESULT|{}|{}", ip, details.to_wire_json()));
        });
    }

    fn render_miner_detail_popup(&mut self, ctx: &egui::Context) {
        if self.edit_rack_map {
            return;
        }

        let Some((rack, slot)) = self.selected_detail_slot else {
            return;
        };

        let target_ip = self
            .rack_slot_target_ip(rack, slot)
            .unwrap_or_else(|| "invalid".to_string());
        let monitor_state = self
            .monitor_results
            .get(&target_ip)
            .copied()
            .unwrap_or(SlotMonitorState::Unknown);
        let assignment = self.slot_assignment(&target_ip).cloned();
        let details = self
            .miner_details
            .get(&target_ip)
            .cloned()
            .unwrap_or_else(|| MinerApiDetails::loading(&target_ip));
        let last_checked = self
            .monitor_last_checked
            .get(&target_ip)
            .cloned()
            .unwrap_or_else(|| "-".to_string());
        let last_seen = self
            .monitor_last_seen
            .get(&target_ip)
            .cloned()
            .unwrap_or_else(|| "-".to_string());

        let mut open = true;
        egui::Window::new("Miner details")
            .id(egui::Id::new("miner_detail_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.set_min_width(370.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(if details.model == "-" {
                                    "Miner".to_string()
                                } else {
                                    details.model.clone()
                                })
                                .size(18.0)
                                .strong()
                                .color(color_text()),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "Rack {:02}  /  Slot {:03}  /  {}",
                                    rack, slot, target_ip
                                ))
                                .small()
                                .color(color_text_muted()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            status_badge(
                                ui,
                                monitor_state.label(),
                                monitor_state_color(monitor_state),
                            );
                        });
                    });

                    ui.add_space(10.0);
                    ui.columns(2, |columns| {
                        detail_metric(&mut columns[0], "HASHRATE", &details.hashrate);
                        detail_metric(&mut columns[1], "POWER", &details.power);
                    });
                    ui.add_space(6.0);
                    ui.columns(2, |columns| {
                        detail_metric(&mut columns[0], "EFFICIENCY", &details.efficiency);
                        detail_metric(&mut columns[1], "UPTIME", &details.uptime);
                    });
                    ui.add_space(6.0);
                    ui.columns(3, |columns| {
                        detail_metric(&mut columns[0], "TEMP", &details.temperature);
                        detail_metric(&mut columns[1], "BOARDS", &details.boards);
                        detail_metric(&mut columns[2], "FANS", &details.fans);
                    });

                    ui.add_space(12.0);
                    section_label(ui, "DEVICE");
                    ui.add_space(4.0);
                    let mac = assignment
                        .as_ref()
                        .map(|row| row.mac.as_str())
                        .filter(|mac| !mac.is_empty())
                        .unwrap_or(&details.mac);
                    egui::Grid::new("miner_detail_grid")
                        .num_columns(2)
                        .spacing(egui::vec2(22.0, 7.0))
                        .show(ui, |ui| {
                            detail_row(ui, "Firmware", &details.firmware);
                            detail_row(ui, "Status", &details.status);
                            detail_row(ui, "Pool", &details.pool);
                            detail_row(ui, "IP address", &target_ip);
                            detail_row(ui, "MAC address", mac);
                            detail_row(ui, "Last seen", &last_seen);
                            detail_row(ui, "Last checked", &last_checked);
                            detail_row(ui, "API updated", &details.updated);
                        });

                    if !details.error.is_empty() {
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(49, 38, 20))
                            .stroke(egui::Stroke::new(
                                1.0,
                                color_warning().linear_multiply(0.55),
                            ))
                            .rounding(egui::Rounding::same(5.0))
                            .inner_margin(egui::Margin::symmetric(9.0, 7.0))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(&details.error)
                                        .small()
                                        .color(color_warning()),
                                );
                            });
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("↻  Refresh").clicked() && valid_ip(&target_ip) {
                            self.request_miner_details(target_ip.clone());
                        }
                        if ui.button("⧉  Copy IP").clicked() {
                            ui.ctx().output_mut(|o| {
                                o.copied_text = target_ip.clone();
                            });
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").clicked() {
                                self.selected_detail_slot = None;
                            }
                        });
                    });
                });
            });

        if !open {
            self.selected_detail_slot = None;
        }
    }

    fn take_next_target_ip(&mut self) -> Option<String> {
        let current = self.next_target_ip?;
        let current_string = current.to_string();

        if self.assigned_target_ips.contains(&current_string) {
            self.status = format!(
                "Target IP conflict prevented: {} already assigned.",
                current_string
            );
            return None;
        }

        self.assigned_target_ips.insert(current_string.clone());
        self.next_target_ip = next_ipv4(current);
        Some(current_string)
    }

    fn block_wrong_subnet_report_if_needed(
        &mut self,
        current_ip: &str,
        mac: &str,
        target_ip: &str,
    ) -> bool {
        if !self.reject_wrong_subnet_reports {
            return false;
        }

        let Some(expected_subnet) = same_subnet_prefix(target_ip) else {
            return false;
        };

        let Some(reported_subnet) = same_subnet_prefix(current_ip) else {
            return false;
        };

        if expected_subnet == reported_subnet {
            return false;
        }

        self.wrong_subnet_popup = Some(WrongSubnetPopup {
            reported_ip: current_ip.to_string(),
            expected_subnet: expected_subnet.clone(),
            reported_subnet: reported_subnet.clone(),
            target_ip: target_ip.to_string(),
            mac: mac.to_string(),
        });

        let msg = format!(
            "Wrong subnet report blocked: current={} target={} mac={} expected {}.x got {}.x. Reset and rescan.",
            current_ip, target_ip, mac, expected_subnet, reported_subnet
        );

        self.status = msg.clone();
        self.log(&msg);
        true
    }

    fn add_miner(&mut self, current_ip: String, mac: String) {
        if self.accept_report_for_armed_slot(current_ip.clone(), mac.clone()) {
            return;
        }

        // If the operator typed a start IP but did not press any extra button,
        // lock it in automatically on the first report.
        if self.redo_target_ip.is_none() && self.next_target_ip.is_none() {
            match parse_ipv4(&self.start_ip_input) {
                Some(ip) => {
                    self.next_target_ip = Some(ip);
                    self.status = format!("Start IP auto-set from field: {}", ip);
                    self.log(&self.status);
                }
                None => {
                    self.status =
                        "Enter a valid Start Target IP before accepting IP Reports.".to_string();
                    self.log(&self.status);
                    return;
                }
            }
        }

        let next_report_target_for_guard = self
            .redo_target_ip
            .clone()
            .or_else(|| self.next_target_ip.map(|ip| ip.to_string()));

        if let Some(target_ip_for_guard) = next_report_target_for_guard {
            if self.block_wrong_subnet_report_if_needed(&current_ip, &mac, &target_ip_for_guard) {
                return;
            }
        }

        // BTC-tool style redo mode:
        // click a row, then the next IP Report overwrites that row's current IP/MAC
        // while preserving the normal next sequential target.
        if let (Some(target_ip), Some(line)) = (self.redo_target_ip.clone(), self.redo_row_line) {
            if let Some(pos) = self
                .rows
                .iter()
                .position(|r| r.line == line && r.target_ip == target_ip)
            {
                // Prevent assigning the same current IP/MAC to two different rows.
                if self
                    .rows
                    .iter()
                    .any(|r| r.line != line && r.current_ip == current_ip)
                {
                    self.status = format!(
                        "Redo blocked: current IP {} is already used on another row.",
                        current_ip
                    );
                    self.redo_target_ip = None;
                    self.redo_row_line = None;
                    return;
                }

                if !mac.is_empty() && self.rows.iter().any(|r| r.line != line && r.mac == mac) {
                    self.status =
                        format!("Redo blocked: MAC {} is already used on another row.", mac);
                    self.redo_target_ip = None;
                    self.redo_row_line = None;
                    return;
                }

                let old_current = self.rows[pos].current_ip.clone();
                let old_mac = self.rows[pos].mac.clone();

                self.rows[pos].current_ip = current_ip.clone();
                self.rows[pos].mac = mac.clone();
                self.rows[pos].status = format!("Replaced {} / {}", old_current, old_mac);

                self.rebuild_assignment_sets();
                if !self.apply_running {
                    self.apply_steps.clear();
                }

                self.status = format!(
                    "Line {} overwritten: {} -> {}. Normal next target remains {}.",
                    line,
                    current_ip,
                    target_ip,
                    self.next_target_ip
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| "None".to_string())
                );

                self.log(&format!(
                    "Redo line {} target={} old_current={} new_current={} old_mac={} new_mac={}",
                    line, target_ip, old_current, current_ip, old_mac, mac
                ));

                self.redo_target_ip = None;
                self.redo_row_line = None;
                return;
            } else {
                self.status = "Redo row no longer exists. Redo cancelled.".to_string();
                self.redo_target_ip = None;
                self.redo_row_line = None;
                return;
            }
        }

        if self.next_target_ip.is_none() {
            self.status = "Set Start Target IP before queueing miners.".to_string();
            return;
        }

        if self.assigned_current_ips.contains(&current_ip) {
            self.status = format!("Duplicate current IP ignored: {}", current_ip);
            return;
        }

        if !mac.is_empty() && self.assigned_macs.contains(&mac) {
            self.status = format!("Duplicate MAC ignored: {}", mac);
            return;
        }

        let Some(target_ip) = self.take_next_target_ip() else {
            return;
        };

        self.assigned_current_ips.insert(current_ip.clone());
        if !mac.is_empty() {
            self.assigned_macs.insert(mac.clone());
        }

        let row = MinerRow {
            line: self.rows.len() + 1,
            current_ip,
            target_ip,
            mac,
            status: "Queued".to_string(),
            apply_order: "".to_string(),
            apply_wave: "".to_string(),
            apply_type: "".to_string(),
            apply_status: "".to_string(),
        };

        self.log(&format!(
            "Queued line {} current={} target={} mac={}",
            row.line, row.current_ip, row.target_ip, row.mac
        ));
        self.status = format!(
            "Queued line {}. Next target IP: {}",
            row.line,
            self.next_target_ip
                .map(|x| x.to_string())
                .unwrap_or_else(|| "None".to_string())
        );
        self.rows.push(row);
        if !self.apply_running {
            self.apply_steps.clear();
        }
        self.scroll_to_bottom_next = true;
    }

    fn select_assignment_row(&mut self, line: usize) {
        self.selected_line = Some(line);

        if let Some(row) = self.rows.iter().find(|row| row.line == line) {
            self.status = format!(
                "Selected line {}: {} -> {}.",
                line, row.current_ip, row.target_ip
            );
        } else {
            self.status = "Selected row no longer exists.".to_string();
            self.selected_line = None;
        }
    }

    fn arm_selected_row_for_redo(&mut self) {
        if self.apply_running {
            self.status = "Wait for the active apply batch before arming a redo.".to_string();
            return;
        }

        let Some(line) = self.selected_line else {
            self.status = "Select a queue row before arming redo.".to_string();
            return;
        };

        if let Some(row) = self.rows.iter().find(|r| r.line == line) {
            self.redo_target_ip = Some(row.target_ip.clone());
            self.redo_row_line = Some(line);
            self.status = format!(
                "Redo armed: next IP Report will overwrite line {} for target {}. Normal sequence will continue after that.",
                line, row.target_ip
            );
        } else {
            self.status = "Selected row no longer exists.".to_string();
            self.selected_line = None;
            self.redo_target_ip = None;
            self.redo_row_line = None;
        }
    }

    fn cancel_redo(&mut self) {
        self.redo_target_ip = None;
        self.redo_row_line = None;
        self.status =
            "Redo cancelled. Next IP Report will use the normal next target IP.".to_string();
    }

    fn should_rewind_next_ip_to(&self, ip: &str) -> bool {
        match (parse_ipv4(ip), self.next_target_ip) {
            (Some(removed_ip), Some(next_ip)) => next_ipv4(removed_ip) == Some(next_ip),
            _ => false,
        }
    }

    fn renumber_rows(&mut self) {
        for (idx, row) in self.rows.iter_mut().enumerate() {
            row.line = idx + 1;
        }
    }

    fn rebuild_assignment_sets(&mut self) {
        self.assigned_current_ips.clear();
        self.assigned_macs.clear();
        self.assigned_target_ips.clear();

        for row in &self.rows {
            if valid_ip(&row.current_ip) {
                self.assigned_current_ips.insert(row.current_ip.clone());
            }
            if !row.mac.is_empty() {
                self.assigned_macs.insert(row.mac.clone());
            }
            self.assigned_target_ips.insert(row.target_ip.clone());
        }

        for skip in &self.skips {
            self.assigned_target_ips.insert(skip.skipped_ip.clone());
        }
    }

    fn delete_selected_entry(&mut self) {
        let Some(line) = self.selected_line else {
            self.status = "Select a miner row to delete first.".to_string();
            return;
        };

        if let Some(pos) = self.rows.iter().position(|r| r.line == line) {
            let removed = self.rows.remove(pos);
            if self.should_rewind_next_ip_to(&removed.target_ip) {
                if let Some(ip) = parse_ipv4(&removed.target_ip) {
                    self.next_target_ip = Some(ip);
                }
            }
            self.renumber_rows();
            self.rebuild_assignment_sets();
            self.apply_steps.clear();
            self.selected_line = None;
            self.redo_target_ip = None;
            self.redo_row_line = None;
            self.status = format!(
                "Deleted line {}: {} -> {}. Target IP {} is released. If it was the latest sequential IP, Next was rewound.",
                line, removed.current_ip, removed.target_ip, removed.target_ip
            );
            self.log(&self.status);
        } else {
            self.status = "Selected row no longer exists.".to_string();
            self.selected_line = None;
        }
    }

    fn undo_last_entry(&mut self) {
        if let Some(removed) = self.rows.pop() {
            if self.should_rewind_next_ip_to(&removed.target_ip) {
                if let Some(ip) = parse_ipv4(&removed.target_ip) {
                    self.next_target_ip = Some(ip);
                }
            }
            self.rebuild_assignment_sets();
            self.apply_steps.clear();
            self.selected_line = None;
            self.redo_target_ip = None;
            self.redo_row_line = None;
            self.status = format!(
                "Deleted last line: {} -> {}. Target IP {} is released. If it was the latest sequential IP, Next was rewound.",
                removed.current_ip, removed.target_ip, removed.target_ip
            );
            self.log(&self.status);
        } else {
            self.status = "No miner rows to delete.".to_string();
        }
    }

    fn skip_next_target(&mut self) {
        if self.next_target_ip.is_none() {
            self.status = "Set Start Target IP before skipping.".to_string();
            return;
        }

        let Some(skipped_ip) = self.take_next_target_ip() else {
            return;
        };
        let reason = if self.skip_reason_input.trim().is_empty() {
            "Skipped".to_string()
        } else {
            self.skip_reason_input.trim().to_string()
        };

        let row = MinerRow {
            line: self.rows.len() + 1,
            current_ip: "SKIPPED".to_string(),
            target_ip: skipped_ip.clone(),
            mac: "".to_string(),
            status: format!("Skipped: {}", reason),
            apply_order: "".to_string(),
            apply_wave: "".to_string(),
            apply_type: "".to_string(),
            apply_status: "Not applied".to_string(),
        };

        self.rows.push(row);
        self.apply_steps.clear();
        self.scroll_to_bottom_next = true;
        self.log(&format!("Skipped target={} reason={}", skipped_ip, reason));
        self.status =
            format!(
            "Skipped {} and added a row. Click that row later to redo/fill it. Next target IP: {}",
            skipped_ip,
            self.next_target_ip.map(|x| x.to_string()).unwrap_or_else(|| "None".to_string())
        );
    }

    fn run_prechecks(&mut self) {
        self.build_safe_order();

        if self.apply_steps.is_empty() {
            self.status =
                "Pre-check: no apply steps needed. Rows are already correct or skipped/unfilled."
                    .to_string();
            self.log(&self.status);
            return;
        }

        let steps = self.apply_steps.clone();
        let stock_user = self.stock_user.clone();
        let stock_password = self.stock_password.clone();
        let vnish_password = self.vnish_password.clone();
        let timeout = self.timeout_secs.max(5);
        let parallel_jobs = self.parallel_jobs.max(1);
        let tx = self.status_tx.clone();

        self.status = format!(
            "Starting pre-check for {} planned apply steps...",
            steps.len()
        );
        self.log(&self.status);

        thread::spawn(move || {
            for chunk in steps.chunks(parallel_jobs) {
                let mut handles = Vec::new();

                for step in chunk.iter().cloned() {
                    let tx_step = tx.clone();
                    let stock_user = stock_user.clone();
                    let stock_password = stock_password.clone();
                    let vnish_password = vnish_password.clone();

                    handles.push(thread::spawn(move || {
                        let result = precheck_any_firmware(
                            &step.current_ip,
                            &stock_user,
                            &stock_password,
                            &vnish_password,
                            timeout,
                        );

                        let _ = tx_step.send(format!("APPLY_RESULT|{}|{}", step.row_line, result));
                    }));
                }

                for h in handles {
                    let _ = h.join();
                }
            }

            let _ = tx.send(
                "Pre-check finished. Review Apply column before running Apply Safe Order."
                    .to_string(),
            );
        });
    }

    fn build_safe_order(&mut self) {
        self.apply_steps.clear();

        // Rows that are already on the correct IP are kept in the occupancy map
        // so another row cannot accidentally target that IP, but they are NOT
        // placed into the pending/apply list.
        let valid_rows: Vec<MinerRow> = self
            .rows
            .iter()
            .filter(|r| valid_ip(&r.current_ip) && valid_ip(&r.target_ip))
            .cloned()
            .collect();

        let already_correct_count = valid_rows
            .iter()
            .filter(|r| r.current_ip == r.target_ip)
            .count();

        let apply_rows: Vec<MinerRow> = valid_rows
            .iter()
            .filter(|r| r.current_ip != r.target_ip)
            .cloned()
            .collect();

        let mut pending: HashMap<usize, MinerRow> =
            apply_rows.iter().map(|r| (r.line, r.clone())).collect();

        // Occupancy includes every valid row, including already-correct rows.
        let mut occupied: HashMap<String, usize> = valid_rows
            .iter()
            .map(|r| (r.current_ip.clone(), r.line))
            .collect();

        let mut used_ips: HashSet<String> =
            valid_rows.iter().map(|r| r.current_ip.clone()).collect();
        for r in &self.rows {
            if valid_ip(&r.target_ip) {
                used_ips.insert(r.target_ip.clone());
            }
        }

        let mut order = 1usize;
        let mut wave = 1usize;

        while !pending.is_empty() {
            let mut progressed = false;
            let keys: Vec<usize> = pending.keys().cloned().collect();

            for line in keys {
                if !pending.contains_key(&line) {
                    continue;
                }
                let row = pending.get(&line).unwrap().clone();
                let target_occupied_by_pending = occupied.get(&row.target_ip).cloned();

                if target_occupied_by_pending.is_none()
                    || target_occupied_by_pending == Some(row.line)
                {
                    self.apply_steps.push(ApplyStep {
                        order,
                        wave,
                        row_line: row.line,
                        current_ip: row.current_ip.clone(),
                        target_ip: row.target_ip.clone(),
                        mac: row.mac.clone(),
                        kind: "DIRECT".to_string(),
                        status: "Planned".to_string(),
                    });
                    order += 1;

                    occupied.remove(&row.current_ip);
                    occupied.insert(row.target_ip.clone(), row.line);
                    pending.remove(&line);
                    progressed = true;
                }
            }

            if progressed {
                wave += 1;
                continue;
            }

            let Some((&line, row)) = pending.iter().next() else {
                break;
            };
            let row = row.clone();

            let Some(park_ip) = parking_ip_for_target(&row.target_ip, &used_ips) else {
                self.status = format!(
                    "No parking IP available in 168-240 range for {}",
                    row.target_ip
                );
                return;
            };

            used_ips.insert(park_ip.clone());

            self.apply_steps.push(ApplyStep {
                order,
                wave,
                row_line: row.line,
                current_ip: row.current_ip.clone(),
                target_ip: park_ip.clone(),
                mac: row.mac.clone(),
                kind: "PARK".to_string(),
                status: "Planned".to_string(),
            });
            order += 1;

            occupied.remove(&row.current_ip);
            occupied.insert(park_ip.clone(), row.line);

            if let Some(p) = pending.get_mut(&line) {
                p.current_ip = park_ip;
            }

            wave += 1;
        }

        self.sync_apply_steps_to_rows();

        let skipped_count = self
            .rows
            .iter()
            .filter(|r| !valid_ip(&r.current_ip))
            .count();
        self.status = format!(
            "Built safe apply order with {} steps. {} already-correct rows skipped. {} skipped/unfilled rows will not be applied.",
            self.apply_steps.len(),
            already_correct_count,
            skipped_count
        );
        self.log(&self.status);
    }

    fn failed_count(&self) -> usize {
        self.apply_steps
            .iter()
            .filter(|s| {
                let st = s.status.to_lowercase();
                !(st == "success" || st == "planned" || st == "waiting" || st == "applying...")
            })
            .count()
    }

    fn export_apply_results_csv(&mut self) {
        let Some(path) = FileDialog::new()
            .set_file_name("blockops_apply_results.csv")
            .save_file()
        else {
            return;
        };

        let mut wtr = match WriterBuilder::new().from_path(&path) {
            Ok(w) => w,
            Err(e) => {
                self.status = format!("Apply results export failed: {}", e);
                return;
            }
        };

        let _ = wtr.write_record([
            "Order", "Wave", "Type", "Line", "From", "To", "MAC", "Status",
        ]);

        for step in &self.apply_steps {
            let _ = wtr.write_record([
                step.order.to_string().as_str(),
                step.wave.to_string().as_str(),
                step.kind.as_str(),
                step.row_line.to_string().as_str(),
                step.current_ip.as_str(),
                step.target_ip.as_str(),
                step.mac.as_str(),
                step.status.as_str(),
            ]);
        }

        let _ = wtr.flush();
        self.status = format!("Exported apply results to {}", path.display());
        self.log(&self.status);
    }

    fn autosave_apply_results(&self) {
        let Ok(mut wtr) = WriterBuilder::new().from_path(&self.apply_results_path) else {
            return;
        };

        let _ = wtr.write_record([
            "Order", "Wave", "Type", "Line", "From", "To", "MAC", "Status",
        ]);

        for step in &self.apply_steps {
            let _ = wtr.write_record([
                step.order.to_string().as_str(),
                step.wave.to_string().as_str(),
                step.kind.as_str(),
                step.row_line.to_string().as_str(),
                step.current_ip.as_str(),
                step.target_ip.as_str(),
                step.mac.as_str(),
                step.status.as_str(),
            ]);
        }

        let _ = wtr.flush();
    }

    fn sync_apply_steps_to_rows(&mut self) {
        let planned_lines: HashSet<usize> =
            self.apply_steps.iter().map(|step| step.row_line).collect();

        for row in &mut self.rows {
            row.apply_order.clear();
            row.apply_wave.clear();
            row.apply_type.clear();
            row.apply_status.clear();

            if row.current_ip == "SKIPPED" {
                row.apply_status = "Not applied".to_string();
            } else if valid_ip(&row.current_ip)
                && valid_ip(&row.target_ip)
                && row.current_ip == row.target_ip
                && !planned_lines.contains(&row.line)
            {
                row.apply_status = "Already correct; skipped".to_string();
            }
        }

        for step in &self.apply_steps {
            if let Some(row) = self.rows.iter_mut().find(|r| r.line == step.row_line) {
                if row.apply_order.is_empty() {
                    row.apply_order = step.order.to_string();
                    row.apply_wave = step.wave.to_string();
                    row.apply_type = step.kind.clone();
                    row.apply_status = step.status.clone();
                } else {
                    row.apply_order = format!("{},{}", row.apply_order, step.order);
                    row.apply_wave = format!("{},{}", row.apply_wave, step.wave);
                    row.apply_type = format!("{},{}", row.apply_type, step.kind);
                    row.apply_status = format!("{} | {}", row.apply_status, step.status);
                }
            }
        }
    }

    fn export_plan(&mut self) {
        let Some(path) = FileDialog::new()
            .set_file_name("blockops_static_ip_plan.csv")
            .save_file()
        else {
            return;
        };

        let mut wtr = match WriterBuilder::new().from_path(&path) {
            Ok(w) => w,
            Err(e) => {
                self.status = format!("Export failed: {}", e);
                return;
            }
        };

        let _ = wtr.write_record([
            "Section",
            "Order/Line",
            "Current IP",
            "Target IP",
            "MAC",
            "Scan Status",
            "Apply Order",
            "Apply Wave",
            "Apply Type",
            "Apply Result",
        ]);

        for row in &self.rows {
            let _ = wtr.write_record([
                "SCAN",
                row.line.to_string().as_str(),
                row.current_ip.as_str(),
                row.target_ip.as_str(),
                row.mac.as_str(),
                row.status.as_str(),
                row.apply_order.as_str(),
                row.apply_wave.as_str(),
                row.apply_type.as_str(),
                row.apply_status.as_str(),
            ]);
        }

        for skip in &self.skips {
            let _ = wtr.write_record([
                "SKIP",
                skip.line.to_string().as_str(),
                "",
                skip.skipped_ip.as_str(),
                "",
                skip.reason.as_str(),
                "",
                "",
                "",
                "",
            ]);
        }

        for step in &self.apply_steps {
            let _ = wtr.write_record([
                "APPLY",
                step.order.to_string().as_str(),
                step.current_ip.as_str(),
                step.target_ip.as_str(),
                step.mac.as_str(),
                step.kind.as_str(),
                step.order.to_string().as_str(),
                step.wave.to_string().as_str(),
                step.kind.as_str(),
                step.status.as_str(),
            ]);
        }

        let _ = wtr.flush();
        self.status = format!("Exported plan to {}", path.display());
        self.log(&self.status);
    }

    fn apply_safe_order(&mut self) {
        if self.apply_running {
            self.apply_queued = true;
            self.status = "An apply is already running. New changes are queued for the next batch."
                .to_string();
            return;
        }

        // Always rebuild the safe apply order right before applying.
        // This prevents stale apply lists and removes the need for a separate Build button.
        self.build_safe_order();

        if self.apply_steps.is_empty() {
            self.status =
                "No apply steps needed. Eligible rows are already correct or skipped/unfilled."
                    .to_string();
            self.log(&self.status);
            self.autosave_apply_results();
            return;
        }

        let steps = self.apply_steps.clone();
        let firmware_modes: HashMap<usize, String> = self
            .rows
            .iter()
            .filter_map(|row| {
                let mode = match self.monitor_results.get(&row.current_ip) {
                    Some(SlotMonitorState::VnishMiner | SlotMonitorState::AuthRequired) => {
                        "vnish_api"
                    }
                    Some(SlotMonitorState::BitmainMiner) => "stock_hiveon_cgi",
                    _ => return None,
                };
                Some((row.line, mode.to_string()))
            })
            .collect();
        let stock_user = self.stock_user.clone();
        let vnish_password = self.vnish_password.clone();
        let stock_password = self.stock_password.clone();
        let netmask = self.netmask.clone();
        let dns1 = self.dns1.clone();
        let dns2 = self.dns2.clone();
        let gateway_override = self.gateway_override.clone();
        let timeout = self.timeout_secs;
        let apply_delay = self.apply_delay_secs;
        let parallel_jobs = self.parallel_jobs.max(1);
        let tx = self.status_tx.clone();

        for s in &mut self.apply_steps {
            s.status = "Waiting".to_string();
        }
        self.apply_running = true;
        self.sync_apply_steps_to_rows();

        thread::spawn(move || {
            let mut waves: BTreeMap<usize, Vec<ApplyStep>> = BTreeMap::new();

            for step in steps {
                waves.entry(step.wave).or_default().push(step);
            }

            for (_wave, wave_steps) in waves {
                for chunk in wave_steps.chunks(parallel_jobs) {
                    let mut handles = Vec::new();

                    for step in chunk.iter().cloned() {
                        let tx_step = tx.clone();
                        let firmware_mode = firmware_modes
                            .get(&step.row_line)
                            .cloned()
                            .unwrap_or_else(|| "auto".to_string());
                        let stock_user = stock_user.clone();
                        let stock_password = stock_password.clone();
                        let vnish_password = vnish_password.clone();
                        let netmask = netmask.clone();
                        let dns1 = dns1.clone();
                        let dns2 = dns2.clone();
                        let gateway_override = gateway_override.clone();

                        handles.push(thread::spawn(move || {
                            let _ = tx_step.send(format!("STEP_STATUS|{}|Applying...", step.order));

                            let result = std::panic::catch_unwind(|| {
                                apply_static_by_mode(
                                    &firmware_mode,
                                    &step.current_ip,
                                    &step.mac,
                                    &step.target_ip,
                                    &stock_user,
                                    &stock_password,
                                    &vnish_password,
                                    &netmask,
                                    &dns1,
                                    &dns2,
                                    &gateway_override,
                                    timeout,
                                )
                            })
                            .unwrap_or_else(|_| {
                                Err("Apply worker stopped unexpectedly".to_string())
                            });

                            let msg = match result {
                                Ok(_) => format!("STEP_STATUS|{}|SUCCESS", step.order),
                                Err(e) => format!("STEP_STATUS|{}|{}", step.order, e),
                            };

                            let _ = tx_step.send(msg);
                        }));
                    }

                    for handle in handles {
                        let _ = handle.join();
                    }
                }

                if apply_delay > 0 {
                    thread::sleep(Duration::from_secs(apply_delay));
                }
            }

            let _ = tx.send("APPLY_SEQUENCE_DONE".to_string());
        });

        self.status = format!(
            "Apply sequence started with up to {} parallel jobs. Results autosave to {}",
            parallel_jobs, self.apply_results_path
        );
        self.autosave_apply_results();
    }

    fn toggle_listener(&mut self) {
        if self.listener_started {
            if let Some(flag) = &self.listener_stop {
                flag.store(true, Ordering::Relaxed);
            }
            self.listener_started = false;
            self.listener_stop = None;
            self.status = "Stopping IP report listener...".to_string();
        } else {
            let stop = Arc::new(AtomicBool::new(false));
            spawn_udp_listener(
                self.listen_port,
                self.report_tx.clone(),
                self.status_tx.clone(),
                stop.clone(),
            );
            self.listener_stop = Some(stop);
            self.listener_started = true;
        }
    }

    fn poll_channels(&mut self) {
        while let Ok(report) = self.report_rx.try_recv() {
            self.add_miner(report.current_ip.clone(), report.mac.clone());
        }

        while let Ok(msg) = self.status_rx.try_recv() {
            if let Some(rest) = msg.strip_prefix("STEP_STATUS|") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                if parts.len() == 2 {
                    if let Ok(order) = parts[0].parse::<usize>() {
                        let mut successful_move = None;
                        if let Some(step) = self.apply_steps.iter_mut().find(|s| s.order == order) {
                            step.status = parts[1].to_string();
                            if parts[1] == "SUCCESS" {
                                successful_move = Some((step.row_line, step.target_ip.clone()));
                            }
                        }
                        if let Some((row_line, new_ip)) = successful_move {
                            if let Some(row) = self.rows.iter_mut().find(|row| row.line == row_line)
                            {
                                row.current_ip = new_ip;
                                row.status = "Static IP accepted by miner".to_string();
                            }
                            self.rebuild_assignment_sets();
                        }
                        self.log(&format!("Apply step {} status: {}", order, parts[1]));
                        self.sync_apply_steps_to_rows();
                        self.autosave_apply_results();
                    }
                }
            } else if msg == "APPLY_SEQUENCE_DONE" {
                self.apply_running = false;
                if self.apply_queued {
                    self.apply_queued = false;
                    self.status = "Apply batch finished. Starting queued changes...".to_string();
                    self.apply_safe_order();
                } else {
                    self.status = "Apply sequence finished.".to_string();
                }
            } else if let Some(rest) = msg.strip_prefix("MONITOR_RESULT|") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                if parts.len() == 2 {
                    let ip = parts[0].to_string();
                    let state = SlotMonitorState::from_wire(parts[1]);
                    let now = Local::now().format("%H:%M:%S").to_string();
                    self.monitor_last_checked.insert(ip.clone(), now.clone());

                    let previous_was_present = self
                        .monitor_results
                        .get(&ip)
                        .is_some_and(|previous| previous.is_present());
                    let hold_previous_state = state == SlotMonitorState::Offline
                        && previous_was_present
                        && *self
                            .monitor_failure_streaks
                            .entry(ip.clone())
                            .and_modify(|streak| *streak = streak.saturating_add(1))
                            .or_insert(1)
                            < 2;

                    if !hold_previous_state {
                        self.monitor_results.insert(ip.clone(), state);
                    }

                    if state.is_present() {
                        self.monitor_failure_streaks.remove(&ip);
                        self.monitor_last_seen.insert(ip.clone(), now);
                    } else if !previous_was_present {
                        self.monitor_failure_streaks.remove(&ip);
                    }

                    let selected_ip = self
                        .selected_detail_slot
                        .and_then(|(rack, slot)| self.rack_slot_target_ip(rack, slot));
                    if selected_ip.as_deref() == Some(ip.as_str())
                        && state.is_present()
                        && !self.details_loading.contains(&ip)
                    {
                        self.request_miner_details(ip);
                    }
                }
            } else if let Some(rest) = msg.strip_prefix("DETAIL_RESULT|") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                if parts.len() == 2 {
                    self.details_loading.remove(parts[0]);
                    if let Some(details) = MinerApiDetails::from_wire_json(parts[1]) {
                        self.miner_details.insert(parts[0].to_string(), details);
                    }
                }
            } else if msg == "MONITOR_DONE" {
                let was_live = self.monitor_live;
                self.monitor_running = false;
                self.monitor_live = false;
                self.monitor_stop = None;
                self.status = if was_live {
                    "Live monitor stopped.".to_string()
                } else {
                    "Monitor scan finished.".to_string()
                };
            } else if msg == "MONITOR_CYCLE_DONE" {
                if self.monitor_live {
                    self.status = format!(
                        "Live monitor updated. Next scan in {} seconds.",
                        self.monitor_interval_secs.max(5)
                    );
                }
            } else {
                self.status = msg;
            }
        }
    }

    fn render_rack_panel(&self, ui: &mut egui::Ui, rack: usize) -> Option<usize> {
        let mut clicked_slot = None;
        let columns = 12usize;
        let rows_per_rack = (self.rack_size + columns - 1) / columns;
        let online_count = (1..=self.rack_size)
            .filter_map(|slot| self.rack_slot_target_ip(rack, slot))
            .filter(|ip| {
                self.monitor_results
                    .get(ip)
                    .copied()
                    .unwrap_or(SlotMonitorState::Unknown)
                    .is_present()
            })
            .count();

        egui::Frame::none()
            .fill(color_surface())
            .stroke(egui::Stroke::new(1.0, color_border()))
            .rounding(egui::Rounding::same(7.0))
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Rack {:02}", rack))
                            .size(15.0)
                            .strong()
                            .color(color_text()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {} online",
                                online_count, self.rack_size
                            ))
                            .small()
                            .color(
                                if online_count == self.rack_size {
                                    color_success()
                                } else {
                                    color_text_muted()
                                },
                            ),
                        );
                    });
                });

                let progress = if self.rack_size == 0 {
                    0.0
                } else {
                    online_count as f32 / self.rack_size as f32
                };
                let (bar_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 3.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(bar_rect, egui::Rounding::same(1.5), color_surface_high());
                if progress > 0.0 {
                    let mut fill_rect = bar_rect;
                    fill_rect.max.x = fill_rect.min.x + bar_rect.width() * progress;
                    ui.painter()
                        .rect_filled(fill_rect, egui::Rounding::same(1.5), color_success());
                }
                ui.add_space(5.0);

                let slot_gap = 3.0;
                let slot_width = ((ui.available_width() - slot_gap * 11.0) / 12.0)
                    .floor()
                    .max(18.0);
                egui::Grid::new(format!("rack_grid_{}", rack))
                    .spacing(egui::vec2(slot_gap, 3.0))
                    .min_col_width(0.0)
                    .show(ui, |ui| {
                        for row_index in 0..rows_per_rack {
                            for column in 0..columns {
                                let slot = row_index * columns + column + 1;
                                if slot > self.rack_size {
                                    ui.allocate_space(egui::vec2(slot_width, 21.0));
                                    continue;
                                }

                                let target_ip =
                                    self.rack_slot_target_ip(rack, slot).unwrap_or_default();
                                let assignment = self.slot_assignment(&target_ip);
                                let selected = self.selected_rack_slot == Some((rack, slot));
                                let monitor_state = self
                                    .monitor_results
                                    .get(&target_ip)
                                    .copied()
                                    .unwrap_or(SlotMonitorState::Unknown);

                                let fill = if selected {
                                    color_warning()
                                } else if let Some(row) = assignment {
                                    if row.current_ip == row.target_ip {
                                        color_success()
                                    } else if row.current_ip == "SKIPPED" {
                                        color_surface_hover()
                                    } else {
                                        egui::Color32::from_rgb(125, 91, 31)
                                    }
                                } else {
                                    match monitor_state {
                                        SlotMonitorState::VnishMiner => color_accent(),
                                        SlotMonitorState::BitmainMiner => {
                                            egui::Color32::from_rgb(36, 156, 139)
                                        }
                                        SlotMonitorState::AuthRequired => {
                                            egui::Color32::from_rgb(118, 92, 186)
                                        }
                                        SlotMonitorState::WebOnline => {
                                            egui::Color32::from_rgb(47, 105, 121)
                                        }
                                        SlotMonitorState::SshOnly => {
                                            egui::Color32::from_rgb(67, 77, 91)
                                        }
                                        SlotMonitorState::Offline => {
                                            egui::Color32::from_rgb(105, 39, 49)
                                        }
                                        SlotMonitorState::Unknown => color_surface_high(),
                                    }
                                };
                                let text_color = if monitor_state == SlotMonitorState::Unknown
                                    && assignment.is_none()
                                    && !selected
                                {
                                    color_text_muted()
                                } else {
                                    color_text()
                                };
                                let stroke = if selected {
                                    egui::Stroke::new(2.0, color_text())
                                } else {
                                    egui::Stroke::new(1.0, fill.linear_multiply(1.12))
                                };
                                let response = slot_cell(
                                    ui,
                                    &slot.to_string(),
                                    egui::vec2(slot_width, 21.0),
                                    fill,
                                    text_color,
                                    stroke,
                                );

                                let mut tip = format!(
                                    "Rack {} / Slot {}\nTarget IP  {}\nStatus  {}",
                                    rack,
                                    slot,
                                    target_ip,
                                    monitor_state.label()
                                );
                                if let Some(row) = assignment {
                                    tip.push_str(&format!(
                                        "\nCurrent IP  {}\nMAC  {}\nCapture  {}\nApply  {}",
                                        row.current_ip, row.mac, row.status, row.apply_status
                                    ));
                                }
                                let was_clicked = response.clicked();
                                response.on_hover_text(tip);
                                if was_clicked {
                                    clicked_slot = Some(slot);
                                }
                            }
                            ui.end_row();
                        }
                    });
            });

        clicked_slot
    }

    fn render_rack_map(&mut self, ui: &mut egui::Ui) {
        let mut clicked_slot: Option<(usize, usize)> = None;

        let scroll_height = ui.available_height().max(320.0);
        egui::ScrollArea::vertical()
            .id_source("rack_map_scroll")
            .max_height(scroll_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for rack_pair_start in (1..=self.rack_count).step_by(2) {
                    ui.columns(2, |columns| {
                        if let Some(slot) = self.render_rack_panel(&mut columns[0], rack_pair_start)
                        {
                            clicked_slot = Some((rack_pair_start, slot));
                        }
                        let second_rack = rack_pair_start + 1;
                        if second_rack <= self.rack_count {
                            if let Some(slot) = self.render_rack_panel(&mut columns[1], second_rack)
                            {
                                clicked_slot = Some((second_rack, slot));
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            });

        if let Some((rack, slot)) = clicked_slot {
            if self.edit_rack_map {
                self.arm_rack_slot(rack, slot);
            } else {
                self.select_rack_slot_details(rack, slot);
            }
        }
    }

    fn render_dashboard_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(color_surface())
            .stroke(egui::Stroke::new(1.0, color_border()))
            .rounding(egui::Rounding::same(7.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.monitor_running,
                            egui::Button::new("↻  Scan all")
                                .fill(color_accent())
                                .stroke(egui::Stroke::NONE),
                        )
                        .clicked()
                    {
                        self.start_monitor_scan(false, None);
                    }

                    ui.separator();
                    toolbar_label(ui, "RACK");
                    let rack_input = ui.add_sized(
                        [44.0, 32.0],
                        egui::TextEdit::singleline(&mut self.monitor_rack_input)
                            .char_limit(2)
                            .hint_text("1-19"),
                    );
                    let scan_rack_clicked = ui
                        .add_enabled(!self.monitor_running, egui::Button::new("Scan rack"))
                        .clicked();
                    let scan_rack_entered = !self.monitor_running
                        && rack_input.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));

                    if scan_rack_clicked || scan_rack_entered {
                        match self.monitor_rack_input.trim().parse::<usize>() {
                            Ok(rack) if (1..=self.rack_count).contains(&rack) => {
                                self.start_monitor_scan(false, Some(rack));
                            }
                            _ => {
                                self.status = format!(
                                    "Enter a rack number from 1 to {}.",
                                    self.rack_count.max(1)
                                );
                            }
                        }
                    }

                    ui.separator();
                    toolbar_label(ui, "LIVE");
                    ui.add(
                        egui::DragValue::new(&mut self.monitor_interval_secs)
                            .clamp_range(5..=600)
                            .suffix(" sec")
                            .speed(1),
                    );

                    if self.monitor_live {
                        let stopping = self
                            .monitor_stop
                            .as_ref()
                            .is_some_and(|flag| flag.load(Ordering::Relaxed));
                        if ui
                            .add_enabled(
                                !stopping,
                                egui::Button::new(if stopping {
                                    "Stopping..."
                                } else {
                                    "■  Stop live"
                                })
                                .fill(color_danger())
                                .stroke(egui::Stroke::NONE),
                            )
                            .clicked()
                        {
                            self.stop_monitor_scan();
                        }
                    } else if ui
                        .add_enabled(
                            !self.monitor_running,
                            egui::Button::new("▶  Start live")
                                .fill(color_success())
                                .stroke(egui::Stroke::NONE),
                        )
                        .clicked()
                    {
                        self.start_monitor_scan(true, None);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if mode_button(ui, "✎  Assign IP", self.edit_rack_map, color_warning())
                            .clicked()
                        {
                            self.edit_rack_map = true;
                        }
                        if mode_button(ui, "⌖  Inspect", !self.edit_rack_map, color_accent())
                            .clicked()
                        {
                            self.edit_rack_map = false;
                            self.armed_target_ip = None;
                        }
                        toolbar_label(ui, "MODE");
                    });
                });

                if self.edit_rack_map {
                    ui.add_space(8.0);
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(49, 38, 20))
                        .stroke(egui::Stroke::new(
                            1.0,
                            color_warning().linear_multiply(0.55),
                        ))
                        .rounding(egui::Rounding::same(5.0))
                        .inner_margin(egui::Margin::symmetric(9.0, 6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let listener_label = if self.listener_started {
                                    "■  Stop listener"
                                } else {
                                    "▶  Start listener"
                                };
                                if ui
                                    .add(egui::Button::new(listener_label).fill(
                                        if self.listener_started {
                                            color_success()
                                        } else {
                                            color_surface_high()
                                        },
                                    ))
                                    .clicked()
                                {
                                    self.toggle_listener();
                                }
                                ui.checkbox(
                                    &mut self.auto_apply_armed_reports,
                                    "Apply immediately",
                                );

                                ui.separator();
                                if let (Some((rack, slot)), Some(target)) =
                                    (self.selected_rack_slot, self.armed_target_ip.as_ref())
                                {
                                    ui.strong(
                                        egui::RichText::new(format!(
                                            "ARMED TARGET   R{:02} / S{:03}   {}",
                                            rack, slot, target
                                        ))
                                        .color(color_warning()),
                                    );
                                    if ui.button("Clear").clicked() {
                                        self.armed_target_ip = None;
                                        self.status = "Armed rack target cancelled.".to_string();
                                    }
                                } else {
                                    ui.label(
                                egui::RichText::new(
                                    "Select a slot, then press IP Report on that physical miner.",
                                )
                                .color(color_warning()),
                            );
                                }
                            })
                        });
                }
            });
    }

    fn render_rack_dashboard(&mut self, ui: &mut egui::Ui) {
        self.render_dashboard_toolbar(ui);
        ui.add_space(8.0);
        self.render_rack_map(ui);
    }

    fn render_capture_session(&mut self, ui: &mut egui::Ui) {
        section_label(ui, "CAPTURE SESSION");
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            status_dot(
                ui,
                if self.listener_started {
                    color_success()
                } else {
                    color_text_muted()
                },
            );
            ui.label(if self.listener_started {
                "IP Report listener active"
            } else {
                "IP Report listener stopped"
            });
            let listener_label = if self.listener_started {
                "Stop"
            } else {
                "Start"
            };
            if ui.button(listener_label).clicked() {
                self.toggle_listener();
            }
        });
        ui.horizontal(|ui| {
            toolbar_label(ui, "FIRST TARGET");
            let start_input = ui.add_sized(
                [126.0, 32.0],
                egui::TextEdit::singleline(&mut self.start_ip_input).hint_text("10.4.1.1"),
            );
            let set_clicked = ui.button("Set").clicked();
            let set_entered =
                start_input.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if set_clicked || set_entered {
                self.set_start_ip();
            }
        });
    }

    fn render_assignment_cursor(&self, ui: &mut egui::Ui, pending_count: usize) {
        section_label(ui, "ASSIGNMENT CURSOR");
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                self.next_target_ip
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "Not set".to_string()),
            )
            .size(20.0)
            .strong()
            .color(if self.next_target_ip.is_some() {
                color_text()
            } else {
                color_text_muted()
            }),
        );
        ui.label(
            egui::RichText::new(format!(
                "{} captured  /  {} ready to apply",
                self.rows.len(),
                pending_count
            ))
            .small()
            .color(color_text_muted()),
        );
    }

    fn render_deployment(
        &mut self,
        ui: &mut egui::Ui,
        pending_count: usize,
        has_pending_changes: bool,
    ) {
        section_label(ui, "DEPLOYMENT");
        ui.add_space(5.0);
        let apply_label = if self.apply_running {
            if self.apply_queued {
                "Applying  /  batch queued"
            } else {
                "Applying changes"
            }
        } else {
            "✓  Apply safe order"
        };
        if ui
            .add_enabled(
                has_pending_changes && !self.apply_running,
                egui::Button::new(apply_label)
                    .fill(color_success())
                    .stroke(egui::Stroke::NONE)
                    .min_size(egui::vec2(170.0, 36.0)),
            )
            .clicked()
        {
            self.apply_safe_order();
        }
        ui.label(
            egui::RichText::new(format!(
                "{} pending  /  {} failed",
                pending_count,
                self.failed_count()
            ))
            .small()
            .color(if self.failed_count() > 0 {
                color_danger()
            } else {
                color_text_muted()
            }),
        );
    }

    fn render_assignment_controls(&mut self, ui: &mut egui::Ui) {
        let has_selection = self.selected_line.is_some();
        let pending_count = self
            .rows
            .iter()
            .filter(|row| {
                valid_ip(&row.current_ip)
                    && valid_ip(&row.target_ip)
                    && row.current_ip != row.target_ip
            })
            .count();
        let has_pending_changes = pending_count > 0;

        egui::Frame::none()
            .fill(color_surface())
            .stroke(egui::Stroke::new(1.0, color_border()))
            .rounding(egui::Rounding::same(7.0))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                if ui.available_width() < 920.0 {
                    self.render_capture_session(ui);
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.columns(2, |columns| {
                        self.render_assignment_cursor(&mut columns[0], pending_count);
                        self.render_deployment(&mut columns[1], pending_count, has_pending_changes);
                    });
                } else {
                    ui.columns(3, |columns| {
                        self.render_capture_session(&mut columns[0]);
                        self.render_assignment_cursor(&mut columns[1], pending_count);
                        self.render_deployment(&mut columns[2], pending_count, has_pending_changes);
                    });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    toolbar_label(ui, "SKIP REASON");
                    ui.add_sized(
                        [116.0, 32.0],
                        egui::TextEdit::singleline(&mut self.skip_reason_input),
                    );
                    if ui
                        .add_enabled(
                            self.next_target_ip.is_some() && !self.apply_running,
                            egui::Button::new("Skip Next"),
                        )
                        .clicked()
                    {
                        self.skip_next_target();
                    }

                    ui.separator();
                    if let Some(line) = self.selected_line {
                        status_badge(ui, &format!("Line {} selected", line), color_accent());
                    } else {
                        toolbar_label(ui, "NO ROW SELECTED");
                    }

                    if ui
                        .add_enabled(
                            has_selection && !self.apply_running,
                            egui::Button::new("Redo selected"),
                        )
                        .clicked()
                    {
                        self.arm_selected_row_for_redo();
                    }
                    if ui
                        .add_enabled(
                            self.redo_target_ip.is_some() && !self.apply_running,
                            egui::Button::new("Cancel redo"),
                        )
                        .clicked()
                    {
                        self.cancel_redo();
                    }
                    if ui
                        .add_enabled(
                            has_selection && !self.apply_running,
                            egui::Button::new("Delete row"),
                        )
                        .clicked()
                    {
                        self.delete_selected_entry();
                    }
                    if ui
                        .add_enabled(
                            !self.rows.is_empty() && !self.apply_running,
                            egui::Button::new("Undo last"),
                        )
                        .clicked()
                    {
                        self.undo_last_entry();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            has_pending_changes && !self.apply_running,
                            egui::Button::new("Pre-check"),
                        )
                        .clicked()
                    {
                        self.run_prechecks();
                    }
                    ui.menu_button("Export  ▾", |ui| {
                        if ui
                            .add_enabled(
                                !self.rows.is_empty(),
                                egui::Button::new("Assignment plan"),
                            )
                            .clicked()
                        {
                            self.export_plan();
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                !self.apply_steps.is_empty(),
                                egui::Button::new("Apply results"),
                            )
                            .clicked()
                        {
                            self.export_apply_results_csv();
                            ui.close_menu();
                        }
                    });
                });

                if let (Some(line), Some(target)) =
                    (self.redo_row_line, self.redo_target_ip.as_ref())
                {
                    ui.add_space(7.0);
                    status_badge(
                        ui,
                        &format!("Redo armed  /  line {} keeps {}", line, target),
                        color_warning(),
                    );
                }
            });
    }

    fn render_assignment_table(&mut self, ui: &mut egui::Ui, max_height: f32) {
        egui::Frame::none()
            .fill(color_surface())
            .stroke(egui::Stroke::new(1.0, color_border()))
            .rounding(egui::Rounding::same(7.0))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Assignment queue")
                                .size(17.0)
                                .strong()
                                .color(color_text()),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "{} miners  /  {} planned steps  /  {} failed",
                                self.rows.len(),
                                self.apply_steps.len(),
                                self.failed_count()
                            ))
                            .small()
                            .color(color_text_muted()),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.auto_scroll_miners, "Follow newest");
                    });
                });
                ui.add_space(8.0);

                if self.rows.is_empty() {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), max_height.max(260.0)),
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            empty_assignment_state(ui, self.listener_started);
                        },
                    );
                    return;
                }

                let row_count = self.rows.len();
                let selected_line = self.selected_line;
                let mut clicked_line = None;
                let mut table = egui_extras::TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .sense(egui::Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .min_scrolled_height(max_height.max(240.0))
                    .max_scroll_height(max_height.max(240.0))
                    .stick_to_bottom(self.auto_scroll_miners)
                    .column(egui_extras::Column::exact(54.0))
                    .column(egui_extras::Column::initial(132.0).at_least(112.0))
                    .column(egui_extras::Column::initial(132.0).at_least(112.0))
                    .column(egui_extras::Column::initial(165.0).at_least(128.0))
                    .column(egui_extras::Column::remainder().at_least(115.0))
                    .column(egui_extras::Column::remainder().at_least(115.0));

                if self.auto_scroll_miners && self.scroll_to_bottom_next && row_count > 0 {
                    table = table.scroll_to_row(row_count - 1, Some(egui::Align::BOTTOM));
                    self.scroll_to_bottom_next = false;
                }

                table
                    .header(34.0, |mut header| {
                        for heading in [
                            "LINE",
                            "CURRENT IP",
                            "TARGET IP",
                            "MAC ADDRESS",
                            "CAPTURE",
                            "APPLY",
                        ] {
                            header.col(|ui| table_heading(ui, heading));
                        }
                    })
                    .body(|body| {
                        body.rows(36.0, row_count, |mut table_row| {
                            let row = &self.rows[table_row.index()];
                            table_row.set_selected(selected_line == Some(row.line));
                            table_row.col(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{:02}", row.line))
                                        .strong()
                                        .color(color_text_muted()),
                                );
                            });
                            table_row.col(|ui| {
                                ui.label(&row.current_ip);
                            });
                            table_row.col(|ui| {
                                ui.label(
                                    egui::RichText::new(&row.target_ip)
                                        .strong()
                                        .color(color_text()),
                                );
                            });
                            table_row.col(|ui| {
                                ui.label(
                                    egui::RichText::new(&row.mac)
                                        .family(egui::FontFamily::Monospace)
                                        .size(12.0),
                                );
                            });
                            table_row.col(|ui| table_status(ui, &row.status));
                            table_row.col(|ui| table_status(ui, &row.apply_status));
                            if table_row.response().clicked() {
                                clicked_line = Some(row.line);
                            }
                        });
                    });

                if let Some(line) = clicked_line {
                    self.select_assignment_row(line);
                }
            });
    }

    fn render_ip_assignment(&mut self, ui: &mut egui::Ui) {
        self.render_assignment_controls(ui);
        ui.add_space(8.0);
        self.render_assignment_table(ui, ui.available_height().max(320.0) - 72.0);
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_source("settings_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Site settings")
                        .size(17.0)
                        .strong()
                        .color(color_text()),
                );
                ui.label(
                    egui::RichText::new("Rack layout, network behavior, and miner access defaults")
                        .small()
                        .color(color_text_muted()),
                );
                ui.add_space(10.0);

                egui::Frame::none()
                    .fill(color_surface())
                    .stroke(egui::Stroke::new(1.0, color_border()))
                    .rounding(egui::Rounding::same(7.0))
                    .inner_margin(egui::Margin::symmetric(12.0, 9.0))
                    .show(ui, |ui| {
                        ui.columns(3, |columns| {
                            settings_summary(
                                &mut columns[0],
                                "ADDRESS SPACE",
                                &format!("10.4.1-{}  /  {} slots", self.rack_count, self.rack_size),
                            );
                            settings_summary(
                                &mut columns[1],
                                "SCAN WORKERS",
                                &self.parallel_jobs.to_string(),
                            );
                            settings_summary(
                                &mut columns[2],
                                "IP REPORT PORT",
                                &self.listen_port.to_string(),
                            );
                        });
                    });
                ui.add_space(10.0);

                let section_stroke = egui::Stroke::new(1.0, color_border());
                ui.columns(2, |columns| {
                    egui::Frame::none()
                        .fill(color_surface())
                        .stroke(section_stroke)
                        .rounding(egui::Rounding::same(7.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(&mut columns[0], |ui| {
                            settings_header(
                                ui,
                                "Rack layout",
                                "Maps each physical slot to its expected address",
                            );
                            ui.add_space(10.0);
                            egui::Grid::new("rack_layout_settings")
                                .num_columns(2)
                                .spacing(egui::vec2(16.0, 8.0))
                                .show(ui, |ui| {
                                    ui.label("Rack 1, Slot 1 IP");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.rack_one_slot_one_ip)
                                            .desired_width(150.0),
                                    );
                                    ui.end_row();
                                    ui.label("Rack count");
                                    ui.add(
                                        egui::DragValue::new(&mut self.rack_count)
                                            .clamp_range(1..=40),
                                    );
                                    ui.end_row();
                                    ui.label("Slots per rack");
                                    ui.add(
                                        egui::DragValue::new(&mut self.rack_size)
                                            .clamp_range(1..=168),
                                    );
                                    ui.end_row();
                                });
                        });

                    columns[0].add_space(8.0);
                    egui::Frame::none()
                        .fill(color_surface())
                        .stroke(section_stroke)
                        .rounding(egui::Rounding::same(7.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(&mut columns[0], |ui| {
                            settings_header(
                                ui,
                                "Monitoring and performance",
                                "Controls scan cadence, concurrency, and apply timing",
                            );
                            ui.add_space(10.0);
                            egui::Grid::new("performance_settings")
                                .num_columns(2)
                                .spacing(egui::vec2(16.0, 8.0))
                                .show(ui, |ui| {
                                    ui.label("Live interval");
                                    ui.add(
                                        egui::DragValue::new(&mut self.monitor_interval_secs)
                                            .clamp_range(5..=600)
                                            .suffix(" sec"),
                                    );
                                    ui.end_row();
                                    ui.label("Parallel jobs");
                                    ui.add(
                                        egui::DragValue::new(&mut self.parallel_jobs)
                                            .clamp_range(1..=64),
                                    );
                                    ui.end_row();
                                    ui.label("Apply timeout");
                                    ui.add(
                                        egui::DragValue::new(&mut self.timeout_secs)
                                            .clamp_range(3..=120)
                                            .suffix(" sec"),
                                    );
                                    ui.end_row();
                                    ui.label("Wave delay");
                                    ui.add(
                                        egui::DragValue::new(&mut self.apply_delay_secs)
                                            .clamp_range(0..=60)
                                            .suffix(" sec"),
                                    );
                                    ui.end_row();
                                });
                        });

                    egui::Frame::none()
                        .fill(color_surface())
                        .stroke(section_stroke)
                        .rounding(egui::Rounding::same(7.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(&mut columns[1], |ui| {
                            settings_header(
                                ui,
                                "IP Report and safety",
                                "Listener and wrong-subnet protection",
                            );
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.label("UDP listener port");
                                ui.add(egui::DragValue::new(&mut self.listen_port).speed(1));
                            });
                            ui.checkbox(
                                &mut self.reject_wrong_subnet_reports,
                                "Reject reports from the wrong subnet",
                            );
                        });

                    columns[1].add_space(8.0);
                    egui::Frame::none()
                        .fill(color_surface())
                        .stroke(section_stroke)
                        .rounding(egui::Rounding::same(7.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(&mut columns[1], |ui| {
                            settings_header(
                                ui,
                                "Miner authentication",
                                "Credentials used for VNISH and stock Bitmain APIs",
                            );
                            ui.add_space(10.0);
                            egui::Grid::new("auth_settings")
                                .num_columns(2)
                                .spacing(egui::vec2(16.0, 8.0))
                                .show(ui, |ui| {
                                    ui.label("VNISH password");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.vnish_password)
                                            .password(true)
                                            .desired_width(160.0),
                                    );
                                    ui.end_row();
                                    ui.label("Bitmain username");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.stock_user)
                                            .desired_width(160.0),
                                    );
                                    ui.end_row();
                                    ui.label("Bitmain password");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.stock_password)
                                            .password(true)
                                            .desired_width(160.0),
                                    );
                                    ui.end_row();
                                });
                        });

                    columns[1].add_space(8.0);
                    egui::Frame::none()
                        .fill(color_surface())
                        .stroke(section_stroke)
                        .rounding(egui::Rounding::same(7.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(&mut columns[1], |ui| {
                            settings_header(
                                ui,
                                "Static network values",
                                "Defaults written during IP assignment",
                            );
                            ui.add_space(10.0);
                            egui::Grid::new("network_settings")
                                .num_columns(2)
                                .spacing(egui::vec2(16.0, 8.0))
                                .show(ui, |ui| {
                                    for (label, value, hint) in [
                                        ("Netmask", &mut self.netmask, "255.255.255.0"),
                                        (
                                            "Gateway",
                                            &mut self.gateway_override,
                                            "blank = auto .254",
                                        ),
                                        ("DNS 1", &mut self.dns1, "1.1.1.1"),
                                        ("DNS 2", &mut self.dns2, "8.8.8.8"),
                                    ] {
                                        ui.label(label);
                                        ui.add(
                                            egui::TextEdit::singleline(value)
                                                .hint_text(hint)
                                                .desired_width(170.0),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                });
            });
    }
}

impl App for BlockOpsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.splash_done {
            self.blockops_splash.show(ctx);
            if self.blockops_splash.is_done() {
                self.splash_done = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            return;
        }

        ctx.request_repaint_after(Duration::from_millis(200));
        self.poll_channels();

        if let Some(popup) = self.wrong_subnet_popup.clone() {
            egui::Window::new("IP Report blocked")
                .collapsible(false)
                .resizable(false)
                .default_width(390.0)
                .show(ctx, |ui| {
                    ui.set_min_width(370.0);
                    status_badge(ui, "WRONG SUBNET", color_danger());
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "The reported miner is outside the armed target subnet",
                        )
                        .size(16.0)
                        .strong()
                        .color(color_text()),
                    );
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    egui::Grid::new("wrong_subnet_details")
                        .num_columns(2)
                        .spacing(egui::vec2(22.0, 7.0))
                        .show(ui, |ui| {
                            detail_row(ui, "Reported IP", &popup.reported_ip);
                            detail_row(ui, "Reported MAC", &popup.mac);
                            detail_row(ui, "Armed target", &popup.target_ip);
                            detail_row(
                                ui,
                                "Expected subnet",
                                &format!("{}.x", popup.expected_subnet),
                            );
                            detail_row(
                                ui,
                                "Reported subnet",
                                &format!("{}.x", popup.reported_subnet),
                            );
                        });
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "Correct the miner network, rescan it, then send IP Report again.",
                        )
                        .color(color_warning()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Dismiss").clicked() {
                            self.wrong_subnet_popup = None;
                        }
                    });
                });
        }

        if self.active_view == AppView::RackDashboard {
            self.render_miner_detail_popup(ctx);
        }

        let fleet_counts =
            (self.active_view == AppView::RackDashboard).then(|| self.monitor_counts());

        egui::SidePanel::left("app_sidebar")
            .exact_width(204.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(color_sidebar())
                    .stroke(egui::Stroke::new(1.0, color_border()))
                    .inner_margin(egui::Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        brand_mark(ui, 38.0);
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("BlockOps")
                                    .size(17.0)
                                    .strong()
                                    .color(color_text()),
                            );
                            ui.label(
                                egui::RichText::new("STATIC IP MANAGER")
                                    .size(10.0)
                                    .strong()
                                    .color(color_text_muted()),
                            );
                        });
                    });
                    ui.add_space(22.0);
                    section_label(ui, "WORKSPACE");
                    ui.add_space(6.0);

                    for (view, icon, label) in [
                        (AppView::RackDashboard, "▦", "Rack dashboard"),
                        (AppView::IpAssignment, "↔", "IP assignment"),
                        (AppView::Settings, "⚙", "Settings"),
                    ] {
                        let selected = self.active_view == view;
                        if nav_button(ui, icon, label, selected).clicked() {
                            self.active_view = view;
                        }
                    }

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        ui.label(
                            egui::RichText::new("VERSION 2.1.1")
                                .size(10.0)
                                .color(color_text_muted()),
                        );
                        ui.add_space(8.0);
                        egui::Frame::none()
                            .fill(color_surface())
                            .stroke(egui::Stroke::new(1.0, color_border()))
                            .rounding(egui::Rounding::same(7.0))
                            .inner_margin(egui::Margin::same(10.0))
                            .show(ui, |ui| {
                                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_min_width(ui.available_width());
                                    section_label(ui, "SITE STATUS");
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        status_dot(
                                            ui,
                                            if self.listener_started {
                                                color_success()
                                            } else {
                                                color_text_muted()
                                            },
                                        );
                                        ui.label(if self.listener_started {
                                            "IP Report listening"
                                        } else {
                                            "IP Report stopped"
                                        });
                                    });
                                    ui.add_space(5.0);
                                    sidebar_stat(
                                        ui,
                                        "Assignment queue",
                                        &self.rows.len().to_string(),
                                    );
                                    sidebar_stat(
                                        ui,
                                        "Planned steps",
                                        &self.apply_steps.len().to_string(),
                                    );
                                    ui.add_space(5.0);
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "10.4.1-{}  /  {} slots per rack",
                                            self.rack_count, self.rack_size
                                        ))
                                        .size(10.5)
                                        .color(color_text_muted()),
                                    );
                                });
                            });
                    });
                });
            });

        egui::TopBottomPanel::top("workspace_header")
            .exact_height(64.0)
            .frame(
                egui::Frame::none()
                    .fill(color_sidebar())
                    .stroke(egui::Stroke::new(1.0, color_border()))
                    .inner_margin(egui::Margin::symmetric(16.0, 9.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(210.0, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.label(
                                egui::RichText::new(self.active_view.label())
                                    .size(19.0)
                                    .strong()
                                    .color(color_text()),
                            );
                            let subtitle = match self.active_view {
                                AppView::RackDashboard => format!(
                                    "{} racks  /  {} managed slots",
                                    self.rack_count,
                                    self.rack_count.saturating_mul(self.rack_size)
                                ),
                                AppView::IpAssignment => format!(
                                    "{} captured miners  /  next target {}",
                                    self.rows.len(),
                                    self.next_target_ip
                                        .map(|ip| ip.to_string())
                                        .unwrap_or_else(|| "not set".to_string())
                                ),
                                AppView::Settings => {
                                    "Site layout, credentials, and network behavior".to_string()
                                }
                            };
                            ui.label(
                                egui::RichText::new(subtitle)
                                    .small()
                                    .color(color_text_muted()),
                            );
                        },
                    );

                    if let Some(counts) = fleet_counts.as_ref() {
                        let status_reserve = if self.listener_started { 220.0 } else { 132.0 };
                        let metrics_width = (ui.available_width() - status_reserve).max(0.0);
                        if metrics_width >= 540.0 {
                            let other_online = counts.auth + counts.web + counts.ssh;
                            ui.allocate_ui_with_layout(
                                egui::vec2(metrics_width, ui.available_height()),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.columns(6, |columns| {
                                        header_metric(
                                            &mut columns[0],
                                            "ONLINE",
                                            counts.present,
                                            color_success(),
                                        );
                                        header_metric(
                                            &mut columns[1],
                                            "VNISH",
                                            counts.vnish,
                                            color_accent(),
                                        );
                                        header_metric(
                                            &mut columns[2],
                                            "BITMAIN",
                                            counts.bitmain,
                                            egui::Color32::from_rgb(43, 177, 157),
                                        );
                                        header_metric(
                                            &mut columns[3],
                                            "OTHER",
                                            other_online,
                                            egui::Color32::from_rgb(93, 139, 158),
                                        );
                                        header_metric(
                                            &mut columns[4],
                                            "OFFLINE",
                                            counts.offline,
                                            color_danger(),
                                        );
                                        header_metric(
                                            &mut columns[5],
                                            "UNSCANNED",
                                            counts.unknown,
                                            color_text_muted(),
                                        );
                                    });
                                },
                            );
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (activity, activity_color) = if self.apply_running {
                            ("APPLYING CHANGES", color_warning())
                        } else if self.monitor_live {
                            ("LIVE MONITORING", color_success())
                        } else if self.monitor_running {
                            ("SCANNING NETWORK", color_accent())
                        } else {
                            ("SYSTEM READY", color_success())
                        };
                        status_badge(ui, activity, activity_color);
                        if self.listener_started {
                            status_badge(ui, "IP REPORT", color_accent());
                        }
                    });
                });
            });

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(32.0)
            .frame(
                egui::Frame::none()
                    .fill(color_sidebar())
                    .stroke(egui::Stroke::new(1.0, color_border()))
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let activity_color = if self.apply_running {
                        color_warning()
                    } else if self.monitor_running || self.listener_started {
                        color_success()
                    } else {
                        color_text_muted()
                    };
                    status_dot(ui, activity_color);
                    ui.label(
                        egui::RichText::new(&self.status)
                            .small()
                            .color(color_text_muted()),
                    );
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(color_app_bg())
                    .inner_margin(egui::Margin::same(12.0)),
            )
            .show(ctx, |ui| match self.active_view {
                AppView::RackDashboard => self.render_rack_dashboard(ui),
                AppView::IpAssignment => self.render_ip_assignment(ui),
                AppView::Settings => self.render_settings(ui),
            });
    }
}

fn slot_cell(
    ui: &mut egui::Ui,
    label: &str,
    size: egui::Vec2,
    fill: egui::Color32,
    text_color: egui::Color32,
    stroke: egui::Stroke,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visible_fill = if response.hovered() {
        fill.linear_multiply(1.16)
    } else {
        fill
    };
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(3.0), visible_fill);
    ui.painter()
        .rect_stroke(rect, egui::Rounding::same(3.0), stroke);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(8.5, egui::FontFamily::Monospace),
        text_color,
    );
    response
}

fn detail_metric(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::none()
        .fill(color_surface_high())
        .stroke(egui::Stroke::new(1.0, color_border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(9.0, 7.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(label)
                        .size(10.0)
                        .strong()
                        .color(color_text_muted()),
                );
                ui.label(
                    egui::RichText::new(value)
                        .size(15.0)
                        .strong()
                        .color(color_text()),
                );
            });
        });
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(10.5)
            .strong()
            .color(color_text_muted()),
    );
}

fn toolbar_label(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(10.0)
            .strong()
            .color(color_text_muted()),
    );
}

fn soft_tint(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

fn status_badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::none()
        .fill(soft_tint(color, 32))
        .stroke(egui::Stroke::new(1.0, soft_tint(color, 105)))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(egui::RichText::new(text).size(10.5).strong().color(color));
            });
        });
}

fn mode_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    color: egui::Color32,
) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(if selected {
            color
        } else {
            color_text_muted()
        }))
        .fill(if selected {
            soft_tint(color, 30)
        } else {
            color_surface_high()
        })
        .stroke(egui::Stroke::new(
            1.0,
            if selected { color } else { color_border() },
        )),
    )
}

fn nav_button(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width(), 42.0],
        egui::Button::new(
            egui::RichText::new(format!("{}   {}", icon, label))
                .size(13.0)
                .color(if selected {
                    color_text()
                } else {
                    color_text_muted()
                }),
        )
        .fill(if selected {
            color_accent_soft()
        } else {
            color_sidebar()
        })
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                soft_tint(color_accent(), 115)
            } else {
                egui::Color32::TRANSPARENT
            },
        )),
    );
    if selected {
        let marker = egui::Rect::from_min_max(
            response.rect.left_top() + egui::vec2(1.0, 7.0),
            response.rect.left_bottom() + egui::vec2(4.0, -7.0),
        );
        ui.painter()
            .rect_filled(marker, egui::Rounding::same(1.5), color_accent());
    }
    response
}

fn brand_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        egui::Rounding::same(8.0),
        egui::Color32::from_rgb(18, 45, 72),
    );
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::same(8.0),
        egui::Stroke::new(1.0, soft_tint(color_accent(), 135)),
    );

    let link_height = size * 0.24;
    let link_width = size * 0.39;
    let center = rect.center();
    let left = egui::Rect::from_center_size(
        center - egui::vec2(size * 0.13, 0.0),
        egui::vec2(link_width, link_height),
    );
    let right = egui::Rect::from_center_size(
        center + egui::vec2(size * 0.13, 0.0),
        egui::vec2(link_width, link_height),
    );
    let stroke = egui::Stroke::new(2.2, color_text());
    ui.painter()
        .rect_stroke(left, egui::Rounding::same(3.0), stroke);
    ui.painter()
        .rect_stroke(right, egui::Rounding::same(3.0), stroke);
    ui.painter().line_segment(
        [
            center - egui::vec2(size * 0.09, 0.0),
            center + egui::vec2(size * 0.09, 0.0),
        ],
        egui::Stroke::new(2.6, color_text()),
    );
}

fn sidebar_stat(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).small().color(color_text_muted()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value)
                    .small()
                    .strong()
                    .color(color_text()),
            );
        });
    });
}

fn header_metric(ui: &mut egui::Ui, label: &str, count: usize, color: egui::Color32) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            status_dot(ui, color);
            ui.label(
                egui::RichText::new(label)
                    .size(10.0)
                    .strong()
                    .color(color_text_muted()),
            );
        });
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(count.to_string())
                    .size(15.0)
                    .strong()
                    .color(color_text()),
            );
        });
    });
}

fn monitor_state_color(state: SlotMonitorState) -> egui::Color32 {
    match state {
        SlotMonitorState::VnishMiner => color_accent(),
        SlotMonitorState::BitmainMiner => egui::Color32::from_rgb(43, 177, 157),
        SlotMonitorState::AuthRequired => egui::Color32::from_rgb(150, 112, 219),
        SlotMonitorState::WebOnline => egui::Color32::from_rgb(93, 160, 181),
        SlotMonitorState::SshOnly => egui::Color32::from_rgb(126, 137, 153),
        SlotMonitorState::Offline => color_danger(),
        SlotMonitorState::Unknown => color_text_muted(),
    }
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).small().color(color_text_muted()));
    ui.label(
        egui::RichText::new(value)
            .small()
            .strong()
            .color(color_text()),
    );
    ui.end_row();
}

fn table_heading(ui: &mut egui::Ui, heading: &str) {
    ui.label(
        egui::RichText::new(heading)
            .size(10.0)
            .strong()
            .color(color_text_muted()),
    );
}

fn table_status(ui: &mut egui::Ui, status: &str) {
    let lower = status.to_ascii_lowercase();
    let color = if lower.contains("fail") || lower.contains("error") || lower.contains("blocked") {
        color_danger()
    } else if lower.contains("applied")
        || lower.contains("complete")
        || lower.contains("correct")
        || lower.contains("success")
        || lower.contains("already")
    {
        color_success()
    } else if lower.contains("pending")
        || lower.contains("waiting")
        || lower.contains("queued")
        || lower.contains("applying")
    {
        color_warning()
    } else if lower.contains("captured") || lower.contains("reported") || lower.contains("ready") {
        color_accent()
    } else {
        color_text_muted()
    };
    status_badge(ui, status, color);
}

fn empty_assignment_state(ui: &mut egui::Ui, listener_started: bool) {
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
        ui.painter()
            .circle_stroke(rect.center(), 20.0, egui::Stroke::new(1.0, color_border()));
        ui.painter().line_segment(
            [
                rect.center() + egui::vec2(-9.0, -5.0),
                rect.center() + egui::vec2(9.0, -5.0),
            ],
            egui::Stroke::new(1.8, color_text_muted()),
        );
        ui.painter().line_segment(
            [
                rect.center() + egui::vec2(-9.0, 5.0),
                rect.center() + egui::vec2(9.0, 5.0),
            ],
            egui::Stroke::new(1.8, color_text_muted()),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("No miners captured")
                .size(17.0)
                .strong()
                .color(color_text()),
        );
        ui.label(
            egui::RichText::new(if listener_started {
                "IP Report listener is active"
            } else {
                "IP Report listener is stopped"
            })
            .color(if listener_started {
                color_success()
            } else {
                color_text_muted()
            }),
        );
    });
}

fn settings_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(15.0)
            .strong()
            .color(color_text()),
    );
    ui.label(
        egui::RichText::new(subtitle)
            .small()
            .color(color_text_muted()),
    );
}

fn settings_summary(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        section_label(ui, label);
        ui.label(
            egui::RichText::new(value)
                .size(15.0)
                .strong()
                .color(color_text()),
        );
    });
}

fn load_app_icon() -> egui::IconData {
    let image = image::load_from_memory(BLOCKOPS_APP_ICON_BYTES)
        .expect("failed to load BlockOps app icon")
        .to_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result<()> {
    center_window_after_startup();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1220.0, 860.0])
            .with_min_inner_size([1050.0, 720.0])
            .with_title("BlockOps Static IP Manager")
            .with_icon(load_app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "BlockOps Static IP Manager",
        options,
        Box::new(|cc| {
            configure_egui(&cc.egui_ctx);
            Box::<BlockOpsApp>::default()
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_mac_formats() {
        assert_eq!(normalize_mac("AA-BB-CC-DD-EE-FF"), "aa:bb:cc:dd:ee:ff");
        assert_eq!(normalize_mac("AABBCCDDEEFF"), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn validates_only_ipv4_addresses() {
        assert!(valid_ip("10.4.19.168"));
        assert!(!valid_ip("10.4.19.999"));
        assert!(!valid_ip("2001:db8::1"));
    }

    #[test]
    fn next_address_stays_inside_assignable_host_range() {
        assert_eq!(
            next_ipv4(Ipv4Addr::new(10, 4, 1, 167)),
            Some(Ipv4Addr::new(10, 4, 1, 168))
        );
        assert_eq!(next_ipv4(Ipv4Addr::new(10, 4, 1, 254)), None);
    }

    #[test]
    fn parking_address_skips_used_hosts() {
        let used = HashSet::from(["10.4.7.168".to_string(), "10.4.7.169".to_string()]);
        assert_eq!(
            parking_ip_for_target("10.4.7.42", &used),
            Some("10.4.7.170".to_string())
        );
    }

    #[test]
    fn gateway_override_supports_auto_and_rejects_invalid_values() {
        assert_eq!(
            gateway_for_target_with_override("10.4.8.12", "").unwrap(),
            "10.4.8.254"
        );
        assert_eq!(
            gateway_for_target_with_override("10.4.8.12", "10.4.8.1").unwrap(),
            "10.4.8.1"
        );
        assert!(gateway_for_target_with_override("10.4.8.12", "gateway").is_err());
    }

    #[test]
    fn parses_ip_report_payload_and_normalizes_mac() {
        let report = parse_report_packet(b"ip=10.4.3.77 mac=AA-BB-CC-DD-EE-FF", "10.4.3.12");
        assert_eq!(report.current_ip, "10.4.3.77");
        assert_eq!(report.mac, "aa:bb:cc:dd:ee:ff");
    }
}
