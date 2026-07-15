#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
use base64::{engine::general_purpose, Engine as _};
use chrono::Local;
use csv::WriterBuilder;
use eframe::{egui, App};
use egui_extras::RetainedImage;
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

static BLOCKOPS_HEADER_LOGO_BYTES: &[u8] = include_bytes!("../assets/blockops_header_logo.png");
static BLOCKOPS_APP_ICON_BYTES: &[u8] = include_bytes!("../assets/blockops_app_icon.png");

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
        let title = wide_null("BlockOps Static IP Tool");

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
    ip.parse::<IpAddr>().is_ok()
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
    header_logo: Option<RetainedImage>,

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

        let header_logo =
            RetainedImage::from_image_bytes("blockops_header_logo", BLOCKOPS_HEADER_LOGO_BYTES)
                .ok();

        Self {
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
            status: "Enter Start Target IP, then press IP Report buttons in physical order."
                .to_string(),
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
            header_logo,

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

    fn render_selected_slot_details(&mut self, ui: &mut egui::Ui) {
        let Some((rack, slot)) = self.selected_detail_slot else {
            ui.label(
                egui::RichText::new("Select a slot to view miner details.")
                    .color(egui::Color32::from_rgb(205, 215, 245)),
            );
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
        let assignment = self.slot_assignment(&target_ip);

        egui::Frame::none()
            .fill(egui::Color32::from_rgb(34, 37, 58))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 76, 108)))
            .rounding(egui::Rounding::same(6.0))
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(
                        egui::RichText::new(format!("Rack {} Slot {}", rack, slot))
                            .color(egui::Color32::WHITE),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("Target {}", target_ip))
                            .color(egui::Color32::from_rgb(205, 215, 245)),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("Monitor {}", monitor_state.label()))
                            .color(egui::Color32::from_rgb(205, 215, 245)),
                    );

                    if let Some(details) = self.miner_details.get(&target_ip) {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("Hashrate {}", details.hashrate))
                                .color(egui::Color32::from_rgb(205, 215, 245)),
                        );
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("Temp {}", details.temperature))
                                .color(egui::Color32::from_rgb(205, 215, 245)),
                        );
                    }

                    if let Some(row) = assignment {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("Current {}", row.current_ip))
                                .color(egui::Color32::from_rgb(205, 215, 245)),
                        );
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("MAC {}", row.mac))
                                .color(egui::Color32::from_rgb(205, 215, 245)),
                        );
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("Apply {}", row.apply_status))
                                .color(egui::Color32::from_rgb(205, 215, 245)),
                        );
                    } else {
                        ui.separator();
                        ui.label(
                            egui::RichText::new("No IP Report captured for this slot yet.")
                                .color(egui::Color32::from_rgb(205, 215, 245)),
                        );
                    }
                });
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
        egui::Window::new(format!("Rack {} Slot {}", rack, slot))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(270.0)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(19, 17, 38))
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} - {}",
                                    if details.model == "-" {
                                        "Miner"
                                    } else {
                                        &details.model
                                    },
                                    target_ip
                                ))
                                .color(egui::Color32::WHITE)
                                .strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} | {}",
                                    details.status, details.firmware
                                ))
                                .color(egui::Color32::from_rgb(198, 203, 232)),
                            );

                            ui.add_space(6.0);
                            ui.separator();

                            ui.horizontal(|ui| {
                                detail_metric(ui, "Hashrate", &details.hashrate);
                                detail_metric(ui, "Consumption", &details.power);
                            });
                            ui.horizontal(|ui| {
                                detail_metric(ui, "Efficiency", &details.efficiency);
                                detail_metric(ui, "Uptime", &details.uptime);
                            });

                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                detail_metric(ui, "Temp", &details.temperature);
                                detail_metric(ui, "Boards", &details.boards);
                                detail_metric(ui, "Fans", &details.fans);
                            });

                            ui.add_space(6.0);
                            ui.separator();

                            ui.label(
                                egui::RichText::new(format!("Pool: {}", details.pool))
                                    .color(egui::Color32::from_rgb(198, 203, 232)),
                            );
                            ui.label(
                                egui::RichText::new(format!("IP: {}", target_ip))
                                    .color(egui::Color32::from_rgb(198, 203, 232)),
                            );
                            let mac = assignment
                                .as_ref()
                                .map(|row| row.mac.as_str())
                                .filter(|mac| !mac.is_empty())
                                .unwrap_or(&details.mac);
                            ui.label(
                                egui::RichText::new(format!("MAC: {}", mac))
                                    .color(egui::Color32::from_rgb(145, 151, 188)),
                            );
                            ui.label(
                                egui::RichText::new(format!("Monitor: {}", monitor_state.label()))
                                    .color(egui::Color32::from_rgb(145, 151, 188)),
                            );
                            ui.label(
                                egui::RichText::new(format!("Last seen: {}", last_seen))
                                    .color(egui::Color32::from_rgb(145, 151, 188)),
                            );
                            ui.label(
                                egui::RichText::new(format!("Last checked: {}", last_checked))
                                    .color(egui::Color32::from_rgb(145, 151, 188)),
                            );
                            ui.label(
                                egui::RichText::new(format!("Updated: {}", details.updated))
                                    .color(egui::Color32::from_rgb(145, 151, 188)),
                            );

                            if !details.error.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(&details.error)
                                        .color(egui::Color32::from_rgb(244, 190, 92)),
                                );
                            }

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button("Refresh").clicked() && valid_ip(&target_ip) {
                                    self.request_miner_details(target_ip.clone());
                                }
                                if ui.button("Copy IP").clicked() {
                                    ui.ctx().output_mut(|o| {
                                        o.copied_text = target_ip.clone();
                                    });
                                }
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

    fn select_row_for_redo(&mut self, line: usize) {
        self.selected_line = Some(line);

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

    fn render_rack_map(&mut self, ui: &mut egui::Ui) {
        let mut clicked_slot: Option<(usize, usize)> = None;
        let cols = 12usize;
        let rows_per_rack = (self.rack_size + cols - 1) / cols;

        egui::Frame::none()
            .fill(egui::Color32::from_rgb(28, 30, 49))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(59, 64, 94)))
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("Rack Map").color(egui::Color32::WHITE));
                    ui.separator();

                    ui.label(egui::RichText::new("Rack 1 Slot 1 IP").color(egui::Color32::from_rgb(205, 215, 245)));
                    ui.add_sized([110.0, 24.0], egui::TextEdit::singleline(&mut self.rack_one_slot_one_ip));

                    ui.label(egui::RichText::new("Racks").color(egui::Color32::from_rgb(205, 215, 245)));
                    ui.add(egui::DragValue::new(&mut self.rack_count).clamp_range(1..=40).speed(1));

                    ui.label(egui::RichText::new("Slots").color(egui::Color32::from_rgb(205, 215, 245)));
                    ui.add(egui::DragValue::new(&mut self.rack_size).clamp_range(1..=168).speed(1));

                    if ui.checkbox(&mut self.edit_rack_map, "Edit mode").changed()
                        && !self.edit_rack_map
                    {
                        self.armed_target_ip = None;
                    }

                    ui.checkbox(&mut self.auto_apply_armed_reports, "Auto apply armed reports");

                    ui.label(egui::RichText::new("Live interval").color(egui::Color32::from_rgb(205, 215, 245)));
                    ui.add(egui::DragValue::new(&mut self.monitor_interval_secs).clamp_range(5..=600).speed(1));

                    if ui
                        .add_enabled(!self.monitor_running, egui::Button::new("Rescan All"))
                        .clicked()
                    {
                        self.start_monitor_scan(false, None);
                    }

                    ui.label(
                        egui::RichText::new("Rack").color(egui::Color32::from_rgb(205, 215, 245)),
                    );
                    let rack_input = ui.add_sized(
                        [44.0, 24.0],
                        egui::TextEdit::singleline(&mut self.monitor_rack_input)
                            .char_limit(2)
                            .hint_text("1-19"),
                    );
                    let scan_rack_clicked = ui
                        .add_enabled(!self.monitor_running, egui::Button::new("Scan Rack"))
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
                                    "Stop Live"
                                }),
                            )
                            .clicked()
                        {
                            self.stop_monitor_scan();
                        }
                    } else if ui
                        .add_enabled(!self.monitor_running, egui::Button::new("Start Live"))
                        .clicked()
                    {
                        self.start_monitor_scan(true, None);
                    }
                });

                let counts = self.monitor_counts();
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    count_chip(ui, "Present", counts.present, counts.total, egui::Color32::from_rgb(75, 151, 116));
                    count_chip(ui, "VNISH", counts.vnish, counts.total, egui::Color32::from_rgb(57, 142, 214));
                    count_chip(ui, "Bitmain", counts.bitmain, counts.total, egui::Color32::from_rgb(42, 166, 147));
                    count_chip(ui, "Auth", counts.auth, counts.total, egui::Color32::from_rgb(126, 101, 211));
                    count_chip(ui, "Web", counts.web, counts.total, egui::Color32::from_rgb(84, 96, 148));
                    count_chip(ui, "SSH", counts.ssh, counts.total, egui::Color32::from_rgb(102, 118, 138));
                    count_chip(ui, "Offline", counts.offline, counts.total, egui::Color32::from_rgb(142, 52, 65));
                    count_chip(ui, "Unknown", counts.unknown, counts.total, egui::Color32::from_rgb(75, 80, 108));
                });

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if self.edit_rack_map {
                        ui.label(egui::RichText::new("Edit mode is ON. Clicking a square arms it for the next IP Report.").color(egui::Color32::from_rgb(255, 212, 92)).strong());
                    } else {
                        ui.label(egui::RichText::new("Dashboard mode. Clicking a square shows miner details.").color(egui::Color32::from_rgb(205, 215, 245)));
                    }

                    if let Some((rack, slot)) = self.selected_rack_slot {
                        let target = self.armed_target_ip.clone()
                            .or_else(|| self.rack_slot_target_ip(rack, slot))
                            .unwrap_or_else(|| "invalid".to_string());
                        ui.separator();
                        ui.label(egui::RichText::new(format!("Selected Rack {} Slot {} -> {}", rack, slot, target)).color(egui::Color32::from_rgb(255, 212, 92)).strong());
                    }

                    if self.armed_target_ip.is_some() && ui.button("Cancel Armed Target").clicked() {
                        self.armed_target_ip = None;
                        self.status = "Armed rack target cancelled.".to_string();
                    }
                });

                self.render_selected_slot_details(ui);

                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .id_source("rack_map_scroll")
                    .max_height(560.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            for rack_pair_start in (1..=self.rack_count).step_by(2) {
                                ui.horizontal_top(|ui| {
                                    for rack in rack_pair_start..=self.rack_count.min(rack_pair_start + 1) {
                                        egui::Frame::none()
                                            .fill(egui::Color32::from_rgb(36, 39, 62))
                                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 76, 108)))
                                            .rounding(egui::Rounding::same(6.0))
                                            .inner_margin(egui::Margin::same(6.0))
                                            .show(ui, |ui| {
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

                                                ui.horizontal(|ui| {
                                                    ui.strong(egui::RichText::new(format!("Rack {}", rack)).color(egui::Color32::WHITE));
                                                    ui.label(egui::RichText::new(format!("{}/{}", online_count, self.rack_size)).color(egui::Color32::from_rgb(180, 190, 220)));
                                                });

                                                egui::Grid::new(format!("rack_grid_{}", rack))
                                                    .spacing(egui::vec2(2.0, 2.0))
                                                    .show(ui, |ui| {
                                                        for row in 0..rows_per_rack {
                                                            for col in 0..cols {
                                                                let slot = row * cols + col + 1;
                                                                if slot > self.rack_size {
                                                                    ui.add_space(22.0);
                                                                    continue;
                                                                }

                                                                let target_ip = self.rack_slot_target_ip(rack, slot).unwrap_or_default();
                                                                let assignment = self.slot_assignment(&target_ip);
                                                                let selected = self.selected_rack_slot == Some((rack, slot));
                                                                let monitor_state = self.monitor_results.get(&target_ip).copied().unwrap_or(SlotMonitorState::Unknown);

                                                                let fill = if selected {
                                                                    egui::Color32::from_rgb(245, 177, 44)
                                                                } else if let Some(row) = assignment {
                                                                    if row.current_ip == row.target_ip {
                                                                        egui::Color32::from_rgb(62, 166, 96)
                                                                    } else if row.current_ip == "SKIPPED" {
                                                                        egui::Color32::from_rgb(105, 109, 132)
                                                                    } else {
                                                                        egui::Color32::from_rgb(226, 171, 55)
                                                                    }
                                                                } else {
                                                                    match monitor_state {
                                                                        SlotMonitorState::VnishMiner => egui::Color32::from_rgb(57, 142, 214),
                                                                        SlotMonitorState::BitmainMiner => egui::Color32::from_rgb(42, 166, 147),
                                                                        SlotMonitorState::AuthRequired => egui::Color32::from_rgb(126, 101, 211),
                                                                        SlotMonitorState::WebOnline => egui::Color32::from_rgb(84, 96, 148),
                                                                        SlotMonitorState::SshOnly => egui::Color32::from_rgb(102, 118, 138),
                                                                        SlotMonitorState::Offline => egui::Color32::from_rgb(142, 52, 65),
                                                                        SlotMonitorState::Unknown => egui::Color32::from_rgb(75, 80, 108),
                                                                    }
                                                                };

                                                                let response = ui.add_sized(
                                                                    [22.0, 20.0],
                                                                    egui::Button::new(egui::RichText::new(slot.to_string()).size(8.0).color(egui::Color32::WHITE))
                                                                        .fill(fill)
                                                                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(93, 100, 137))),
                                                                );

                                                                let mut tip = format!("Rack {} Slot {}\nTarget IP: {}\nMonitor: {}", rack, slot, target_ip, monitor_state.label());
                                                                if let Some(row) = assignment {
                                                                    tip.push_str(&format!("\nCurrent IP: {}\nMAC: {}\nStatus: {}\nApply: {}", row.current_ip, row.mac, row.status, row.apply_status));
                                                                }
                                                                let was_clicked = response.clicked();
                                                                response.on_hover_text(tip);

                                                                if was_clicked {
                                                                    clicked_slot = Some((rack, slot));
                                                                }
                                                            }
                                                            ui.end_row();
                                                        }
                                                    });
                                            });
                                        ui.add_space(8.0);
                                    }
                                });
                                ui.add_space(6.0);
                            }
                        });
                    });
            });

        if let Some((rack, slot)) = clicked_slot {
            if self.edit_rack_map {
                self.arm_rack_slot(rack, slot);
            } else {
                self.select_rack_slot_details(rack, slot);
            }
        }
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

        self.poll_channels();

        if let Some(popup) = self.wrong_subnet_popup.clone() {
            egui::Window::new("Wrong subnet IP Report blocked")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("This IP Report was NOT accepted.");
                    ui.separator();
                    ui.label(format!("Reported current IP: {}", popup.reported_ip));
                    ui.label(format!("Reported MAC: {}", popup.mac));
                    ui.label(format!("Next target IP: {}", popup.target_ip));
                    ui.label(format!("Expected subnet: {}.x", popup.expected_subnet));
                    ui.label(format!("Reported subnet: {}.x", popup.reported_subnet));
                    ui.separator();
                    ui.label("Reset that miner to DHCP / correct subnet, rescan, then press IP Report again.");
                    ui.horizontal(|ui| {
                        if ui.button("Reset and rescan").clicked() {
                            self.wrong_subnet_popup = None;
                        }
                        if ui.button("Dismiss").clicked() {
                            self.wrong_subnet_popup = None;
                        }
                    });
                });
        }

        self.render_miner_detail_popup(ctx);

        egui::TopBottomPanel::top("brand_header")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(8, 20, 70))
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(logo) = &self.header_logo {
                        logo.show_size(ui, egui::vec2(300.0, 60.0));
                    } else {
                        ui.heading(
                            egui::RichText::new("BlockOps Mining").color(egui::Color32::WHITE),
                        );
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);

                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Multi-Firmware Static IP Tool")
                                .size(24.0)
                                .strong()
                                .color(egui::Color32::WHITE),
                        );
                        ui.label(
                            egui::RichText::new(
                                "VNISH + Bitmain/Hiveon support with safe rack assignment",
                            )
                            .color(egui::Color32::from_rgb(205, 215, 245)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let listener_label = if self.listener_started {
                            "Stop Listening"
                        } else {
                            "Start Listening"
                        };
                        if ui
                            .add_sized([150.0, 34.0], egui::Button::new(listener_label))
                            .clicked()
                        {
                            self.toggle_listener();
                        }
                    });
                });
            });

        egui::TopBottomPanel::bottom("bottom")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(238, 242, 248))
                    .inner_margin(egui::Margin::same(6.0)),
            )
            .show(ctx, |ui| {
                ui.label(&self.status);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(245, 247, 251)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(6.0);
                        self.render_rack_map(ui);

                        ui.add_space(10.0);

                        ui.horizontal_top(|ui| {
                            let card_stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(185, 198, 220));
                            let card_margin = egui::Margin::same(12.0);

                            egui::Frame::none()
                                .fill(egui::Color32::WHITE)
                                .stroke(card_stroke)
                                .rounding(egui::Rounding::same(8.0))
                                .inner_margin(card_margin)
                                .show(ui, |ui| {
                                    ui.set_min_size(egui::vec2(340.0, 165.0));
                                    ui.set_max_width(360.0);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.heading("1. Connection");
                                            help_icon(ui, "Connection settings control the UDP listener and how fast the app pushes miner changes. Advanced settings can override gateway for sites that do not use target-subnet .254.");
                                        });
                                        ui.separator();

                                        ui.horizontal(|ui| {
                                            ui.label("UDP port");
                                            ui.add(egui::DragValue::new(&mut self.listen_port).speed(1));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Parallel jobs");
                                            ui.add(egui::DragValue::new(&mut self.parallel_jobs).clamp_range(1..=64).speed(1));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Timeout");
                                            ui.add(egui::DragValue::new(&mut self.timeout_secs).speed(1));
                                            ui.label("Delay");
                                            ui.add(egui::DragValue::new(&mut self.apply_delay_secs).speed(1));
                                        });

                                        ui.separator();
                                        ui.checkbox(&mut self.reject_wrong_subnet_reports, "Reject wrong-subnet reports");

                                        ui.separator();
                                        ui.collapsing("Advanced auth / network settings", |ui| {
                                            ui.label("Mode: Auto VNISH + Bitmain Stock/Hiveon");
                                            ui.horizontal(|ui| {
                                                ui.label("VNISH pwd");
                                                ui.add(egui::TextEdit::singleline(&mut self.vnish_password).password(true).desired_width(80.0));
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label("Stock user");
                                                ui.add(egui::TextEdit::singleline(&mut self.stock_user).desired_width(80.0));
                                                ui.label("Stock pwd");
                                                ui.add(egui::TextEdit::singleline(&mut self.stock_password).password(true).desired_width(80.0));
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label("Netmask");
                                                ui.add(egui::TextEdit::singleline(&mut self.netmask).desired_width(120.0));
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label("Gateway");
                                                ui.add(egui::TextEdit::singleline(&mut self.gateway_override).hint_text("blank = auto .254").desired_width(130.0));
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label("DNS1");
                                                ui.add(egui::TextEdit::singleline(&mut self.dns1).desired_width(95.0));
                                                ui.label("DNS2");
                                                ui.add(egui::TextEdit::singleline(&mut self.dns2).desired_width(95.0));
                                            });
                                        });
                                    });
                                });

                            ui.add_space(10.0);

                            egui::Frame::none()
                                .fill(egui::Color32::WHITE)
                                .stroke(card_stroke)
                                .rounding(egui::Rounding::same(8.0))
                                .inner_margin(card_margin)
                                .show(ui, |ui| {
                                    ui.set_min_size(egui::vec2(455.0, 165.0));
                                    ui.set_max_width(485.0);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.heading("2. Capture rack order");
                                            help_icon(ui, "Type the first target IP, start listening, then press miner IP Report buttons in physical order. The app uses the typed start IP automatically on the first report.");
                                        });
                                        ui.separator();

                                        ui.horizontal(|ui| {
                                            ui.label("Start target IP");
                                            ui.add_sized([145.0, 24.0], egui::TextEdit::singleline(&mut self.start_ip_input).hint_text("10.5.9.1"));
                                            if ui.button("Reset Next").clicked() {
                                                self.set_start_ip();
                                            }
                                        });

                                        ui.horizontal(|ui| {
                                            ui.label("Next target");
                                            ui.strong(self.next_target_ip.map(|ip| ip.to_string()).unwrap_or_else(|| "Uses start IP on first report".to_string()));
                                        });

                                        ui.separator();

                                        ui.horizontal(|ui| {
                                            ui.label("Skip reason");
                                            ui.add_sized([180.0, 24.0], egui::TextEdit::singleline(&mut self.skip_reason_input));
                                            if ui.button("Skip Next").clicked() { self.skip_next_target(); }
                                        });

                                        ui.separator();

                                        ui.horizontal(|ui| {
                                            if ui.button("Delete Selected").clicked() { self.delete_selected_entry(); }
                                            if ui.button("Undo Last").clicked() { self.undo_last_entry(); }
                                            if ui.button("Cancel Redo").clicked() { self.cancel_redo(); }
                                        });
                                    });
                                });

                            ui.add_space(10.0);

                            egui::Frame::none()
                                .fill(egui::Color32::WHITE)
                                .stroke(card_stroke)
                                .rounding(egui::Rounding::same(8.0))
                                .inner_margin(card_margin)
                                .show(ui, |ui| {
                                    ui.set_min_size(egui::vec2(330.0, 165.0));
                                    ui.set_max_width(360.0);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.heading("3. Apply");
                                            help_icon(ui, "Apply Safe Order automatically rebuilds the safe order first, skips miners already on the correct IP, then applies only needed changes. Pre-check tests planned targets before applying.");
                                        });
                                        ui.separator();

                                        if ui
                                            .add_enabled_ui(!self.apply_running, |ui| {
                                                ui.add_sized(
                                                    [220.0, 34.0],
                                                    egui::Button::new("Apply Safe Order"),
                                                )
                                            })
                                            .inner
                                            .clicked()
                                        {
                                            self.apply_safe_order();
                                        }

                                        if self.apply_running {
                                            ui.label(if self.apply_queued {
                                                "Applying now; another batch is queued."
                                            } else {
                                                "Applying changes..."
                                            });
                                        }

                                        if ui.add_sized([220.0, 28.0], egui::Button::new("Pre-check Planned Changes")).clicked() {
                                            self.run_prechecks();
                                        }

                                        ui.horizontal(|ui| {
                                            if ui.button("Export Plan CSV").clicked() { self.export_plan(); }
                                            if ui.button("Export Results").clicked() { self.export_apply_results_csv(); }
                                        });

                                        ui.separator();

                                        ui.horizontal(|ui| {
                                            ui.label("Parking");
                                            ui.strong("168-240");
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Steps");
                                            ui.strong(self.apply_steps.len().to_string());
                                            ui.separator();
                                            ui.label("Failed");
                                            ui.strong(self.failed_count().to_string());
                                        });
                                    });
                                });
                        });

                        ui.add_space(8.0);

                        egui::Frame::none()
                            .fill(egui::Color32::WHITE)
                            .stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(185, 198, 220)))
                            .rounding(egui::Rounding::same(8.0))
                            .inner_margin(egui::Margin::same(10.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.heading("Miners / scan + apply results");
                                    ui.separator();
                                    if let Some(line) = self.selected_line {
                                        ui.label(format!("Selected line: {}", line));
                                    } else {
                                        ui.label("Selected line: none");
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.checkbox(&mut self.auto_scroll_miners, "Auto-scroll");
                                    });
                                });

                                egui::ScrollArea::both()
                                    .id_source("miner_scroll")
                                    .max_height(590.0)
                                    .auto_shrink([false, false])
                                    .stick_to_bottom(self.auto_scroll_miners)
                                    .show(ui, |ui| {
                                        egui::Grid::new("miner_grid").striped(true).min_col_width(95.0).spacing(egui::vec2(16.0, 6.0)).show(ui, |ui| {
                                            ui.strong("Line");
                                            ui.strong("Current IP");
                                            ui.strong("Target IP");
                                            ui.strong("MAC");
                                            ui.strong("Status");
                                            ui.strong("Apply");
                                            ui.end_row();

                                            for _ in 0..6 {
                                                ui.separator();
                                            }
                                            ui.end_row();

                                            let mut clicked_line: Option<usize> = None;

                                            for row in &self.rows {
                                                let selected = self.selected_line == Some(row.line);

                                                if ui.selectable_label(selected, row.line.to_string()).clicked() {
                                                    clicked_line = Some(row.line);
                                                }
                                                if ui.selectable_label(selected, &row.current_ip).clicked() {
                                                    clicked_line = Some(row.line);
                                                }
                                                if ui.selectable_label(selected, &row.target_ip).clicked() {
                                                    clicked_line = Some(row.line);
                                                }
                                                if ui.selectable_label(selected, &row.mac).clicked() {
                                                    clicked_line = Some(row.line);
                                                }
                                                if ui.selectable_label(selected, &row.status).clicked() {
                                                    clicked_line = Some(row.line);
                                                }
                                                if ui.selectable_label(selected, &row.apply_status).clicked() {
                                                    clicked_line = Some(row.line);
                                                }
                                                ui.end_row();

                                                for _ in 0..6 {
                                                    ui.separator();
                                                }
                                                ui.end_row();
                                            }

                                            if let Some(line) = clicked_line {
                                                self.select_row_for_redo(line);
                                            }

                                            if self.auto_scroll_miners && self.scroll_to_bottom_next {
                                                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                                                self.scroll_to_bottom_next = false;
                                            }
                                        });
                                    });
                            });

                                    ui.add_space(8.0);
                    });
            });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn help_icon(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new("?")
                .strong()
                .color(egui::Color32::from_rgb(20, 43, 110))
                .background_color(egui::Color32::from_rgb(230, 235, 248)),
        )
        .sense(egui::Sense::hover()),
    )
    .on_hover_text(text);
}

fn detail_metric(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::none()
        .fill(egui::Color32::from_rgb(31, 28, 57))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(egui::Margin::symmetric(7.0, 5.0))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(label)
                        .size(10.0)
                        .color(egui::Color32::from_rgb(170, 176, 210)),
                );
                ui.label(
                    egui::RichText::new(value)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
            });
        });
}

fn count_chip(ui: &mut egui::Ui, label: &str, count: usize, total: usize, color: egui::Color32) {
    let percent = if total > 0 {
        (count as f32 / total as f32) * 100.0
    } else {
        0.0
    };

    egui::Frame::none()
        .fill(egui::Color32::from_rgb(36, 39, 62))
        .stroke(egui::Stroke::new(1.0, color))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(egui::Margin::symmetric(7.0, 4.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("{} {} ({:.0}%)", label, count, percent))
                    .color(egui::Color32::WHITE)
                    .strong(),
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
            .with_title("BlockOps Static IP Tool")
            .with_icon(load_app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "BlockOps Static IP Tool",
        options,
        Box::new(|_cc| Box::<BlockOpsApp>::default()),
    )
}
