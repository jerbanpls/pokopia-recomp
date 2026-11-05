// SPDX-License-Identifier: GPL-3.0-or-later
//! What a relay host tells visitors: which of this PC's IPv4 addresses to give out, and whether Windows will let
//! visitors in. Read only; nothing here changes the network profile or the firewall.
//!
//! Idea and the phone-hotspot bug from pokopia-native (MIT), windows/src/coop/coop_plat.c `cplat_local_addrs` /
//! `public_adapters` @7c3655d: a PC advertised the address of an iPhone's USB hotspot adapter (a Public network), and the
//! visitor's connection was blocked. The ranking below is our own: it is a pure function over [`Adapter`] so it is tested
//! without a network.

use serde::Serialize;
use std::net::Ipv4Addr;

pub const PUBLIC_WARNING: &str = "Windows treats this network as Public, so other PCs may not reach you. Set it to Private in Settings > Network.";
#[cfg(windows)]
pub const FIREWALL_HINT: &str = "If Windows Firewall asks about this program, allow it on Private networks. If it was answered wrongly once, remove the Block rules for the program in Windows Defender Firewall > Allow an app.";
#[cfg(not(windows))]
pub const FIREWALL_HINT: &str = "If visitors cannot connect, the firewall may block the port (39660 unless you changed it). With firewalld (Fedora, Bazzite): sudo firewall-cmd --add-port=39660/tcp --add-port=39660/udp";

/// IfType values we care about (IANA).
pub const IF_WIFI: u32 = 71;
pub const IF_PPP: u32 = 23;
pub const IF_TUNNEL: u32 = 131;
pub const IF_LOOPBACK: u32 = 24;

/// One IPv4 address on one adapter, as read from the OS.
#[derive(Clone, Debug)]
pub struct Adapter {
    pub friendly_name: String,
    pub description: String,
    pub if_type: u32,
    pub up: bool,
    pub ip: Ipv4Addr,
    /// The adapter has an IPv4 default gateway.
    pub has_gateway: bool,
    /// Interface metric (lower wins); only compared between adapters with a gateway.
    pub metric: u32,
    /// Some(true): Windows profile Public. Some(false): Private or Domain. None: unknown.
    pub public: Option<bool>,
}

impl Adapter {
    fn name(&self) -> String {
        if self.friendly_name.is_empty() { self.description.clone() } else { self.friendly_name.clone() }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LanAddr {
    pub ip: String,
    pub adapter: String,
    /// Holds the default route.
    pub default_route: bool,
    pub public_profile: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OverlayAddr {
    pub ip: String,
    pub adapter: String,
    /// "internet via Tailscale" / "internet via ZeroTier"
    pub label: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Skipped {
    pub ip: String,
    pub adapter: String,
    pub reason: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct LanReport {
    /// Best first: show lan[0] to visitors on this network.
    pub lan: Vec<LanAddr>,
    pub overlay: Vec<OverlayAddr>,
    pub skipped: Vec<Skipped>,
    /// Set when the address in lan[0] sits on a Public Windows network.
    pub warning: Option<String>,
    /// Set together with a warning, or when there is no usable address.
    pub firewall_hint: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Class {
    Lan,
    Overlay(&'static str),
    Skip(&'static str),
}

fn has_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

fn classify(a: &Adapter) -> Class {
    let text = format!("{} {}", a.friendly_name, a.description).to_ascii_lowercase();
    let o = a.ip.octets();
    if !a.up {
        return Class::Skip("adapter is down");
    }
    if a.ip.is_loopback() || a.if_type == IF_LOOPBACK {
        return Class::Skip("loopback");
    }
    if a.ip.is_link_local() {
        return Class::Skip("link-local (169.254), no DHCP answer");
    }
    if a.ip.is_unspecified() {
        return Class::Skip("no address");
    }
    if text.contains("tailscale") {
        return Class::Overlay("internet via Tailscale");
    }
    if text.contains("zerotier") {
        return Class::Overlay("internet via ZeroTier");
    }
    if has_any(&text, &["rndis", "remote ndis", "apple mobile device", "iphone", "android", "tether"]) {
        return Class::Skip("USB-tethered phone");
    }
    // iPhone Personal Hotspot range over a wired (USB) link; a PC that joined it over Wi-Fi is a real, if tiny, LAN
    if o[0] == 172 && o[1] == 20 && o[2] == 10 && o[3] < 16 && a.if_type != IF_WIFI {
        return Class::Skip("USB-tethered phone");
    }
    if has_any(
        &text,
        &[
            "vethernet", "hyper-v", "wsl", "virtualbox", "vmware", "vmnet", "host-only", "docker", "npcap", "loopback adapter",
            "bluetooth", "pseudo", "wi-fi direct", "hosted network",
        ],
    ) {
        return Class::Skip("virtual adapter");
    }
    if has_any(
        &text,
        &["wintun", "tap-windows", "tap adapter", "openvpn", "wireguard", "nordlynx", "vpn", "anyconnect", "globalprotect", "fortinet", "tun0", "utun"],
    ) || a.if_type == IF_PPP
        || a.if_type == IF_TUNNEL
    {
        return Class::Skip("VPN or tunnel adapter");
    }
    Class::Lan
}

/// Sort key, smaller first: default-route holders, then RFC 1918 before other ranges, then lower metric, then address.
fn lan_key(a: &Adapter) -> (bool, bool, u32, u32) {
    (!a.has_gateway, !a.ip.is_private(), if a.has_gateway { a.metric } else { u32::MAX }, u32::from(a.ip))
}

/// Pure ranking over the adapter list. The default route is the gateway-holding LAN adapter with the lowest metric.
/// A Public profile does not demote an address (the visitor still has to use it); it raises the warning instead.
pub fn rank(adapters: &[Adapter]) -> LanReport {
    let mut rep = LanReport::default();
    let mut lan: Vec<&Adapter> = Vec::new();
    for a in adapters {
        match classify(a) {
            Class::Lan => lan.push(a),
            Class::Overlay(label) => rep.overlay.push(OverlayAddr { ip: a.ip.to_string(), adapter: a.name(), label: label.to_string() }),
            Class::Skip(why) => {
                // an adapter that is down or empty is not worth listing
                if a.up && !a.ip.is_unspecified() {
                    rep.skipped.push(Skipped { ip: a.ip.to_string(), adapter: a.name(), reason: why.to_string() });
                }
            }
        }
    }
    lan.sort_by_key(|a| lan_key(a));
    let mut first = true;
    for a in &lan {
        let default_route = a.has_gateway && std::mem::take(&mut first);
        if !a.has_gateway {
            first = false;
        }
        rep.lan.push(LanAddr { ip: a.ip.to_string(), adapter: a.name(), default_route, public_profile: a.public == Some(true) });
    }
    if rep.lan.first().map_or(false, |l| l.public_profile) {
        rep.warning = Some(PUBLIC_WARNING.to_string());
    }
    if rep.warning.is_some() || rep.lan.is_empty() {
        rep.firewall_hint = Some(FIREWALL_HINT.to_string());
    }
    rep
}

/// Reads this PC's adapters (Windows: IP Helper; Linux: getifaddrs) and ranks them.
pub fn report() -> LanReport {
    rank(&sys::adapters())
}

/// Log block for `--host`.
pub fn log_lines(rep: &LanReport, port: u16) -> Vec<String> {
    let mut v = Vec::new();
    match rep.lan.first() {
        Some(a) => v.push(format!("Give visitors this address: {}:{} ({})", a.ip, port, a.adapter)),
        None => v.push("No usable LAN address found on this PC.".to_string()),
    }
    for a in rep.lan.iter().skip(1) {
        v.push(format!("  other LAN address: {}:{} ({}{})", a.ip, port, a.adapter, if a.public_profile { ", Public network" } else { "" }));
    }
    for o in &rep.overlay {
        v.push(format!("  {}: {}:{} ({})", o.label, o.ip, port, o.adapter));
    }
    for s in &rep.skipped {
        v.push(format!("  not offered: {} on {} ({})", s.ip, s.adapter, s.reason));
    }
    if let Some(w) = &rep.warning {
        v.push(format!("WARNING: {}", w));
    }
    if let Some(h) = &rep.firewall_hint {
        v.push(format!("Hint: {}", h));
    }
    v
}

#[cfg(windows)]
mod sys {
    use super::*;
    use std::collections::HashMap;
    use windows::core::GUID;
    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::NetworkListManager::{INetworkListManager, NetworkListManager, NLM_NETWORK_CATEGORY_PUBLIC};
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED};

    fn guid_str(g: &GUID) -> String {
        format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7]
        )
    }

    /// adapter id -> is Public, from INetworkListManager (the API behind Settings > Network). Empty if COM fails.
    fn profiles() -> HashMap<String, bool> {
        let mut out = HashMap::new();
        unsafe {
            let init = CoInitializeEx(None, COINIT_MULTITHREADED);
            if let Ok(mgr) = CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, CLSCTX_ALL) {
                if let Ok(en) = mgr.GetNetworkConnections() {
                    loop {
                        let mut c = [None];
                        let mut got = 0u32;
                        if en.Next(&mut c, Some(&mut got)).is_err() || got == 0 {
                            break;
                        }
                        let Some(conn) = c[0].take() else { break };
                        if let (Ok(id), Ok(net)) = (conn.GetAdapterId(), conn.GetNetwork()) {
                            if let Ok(cat) = net.GetCategory() {
                                // an adapter with several connections: Public wins
                                let e = out.entry(guid_str(&id)).or_insert(false);
                                *e |= cat == NLM_NETWORK_CATEGORY_PUBLIC;
                            }
                        }
                    }
                }
            }
            if init.is_ok() {
                CoUninitialize();
            }
        }
        out
    }

    pub fn adapters() -> Vec<Adapter> {
        let prof = profiles();
        let mut out = Vec::new();
        unsafe {
            let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
            let mut len = 16384u32;
            let mut buf: Vec<u64> = Vec::new();
            let mut r = ERROR_BUFFER_OVERFLOW.0;
            for _ in 0..3 {
                if r != ERROR_BUFFER_OVERFLOW.0 {
                    break;
                }
                buf = vec![0u64; (len as usize + 7) / 8];
                r = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut len);
            }
            if r != NO_ERROR.0 {
                return out;
            }
            let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
            while !p.is_null() {
                let a = &*p;
                let id = a.AdapterName.to_string().unwrap_or_default();
                let friendly = a.FriendlyName.to_string().unwrap_or_default();
                let desc = a.Description.to_string().unwrap_or_default();
                let has_gw = !a.FirstGatewayAddress.is_null();
                let public = prof.iter().find(|(k, _)| k.eq_ignore_ascii_case(&id)).map(|(_, v)| *v);
                let mut u = a.FirstUnicastAddress;
                while !u.is_null() {
                    let ua = &*u;
                    let sa = ua.Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let sin = &*(sa as *const SOCKADDR_IN);
                        let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr));
                        out.push(Adapter {
                            friendly_name: friendly.clone(),
                            description: desc.clone(),
                            if_type: a.IfType,
                            up: a.OperStatus == IfOperStatusUp,
                            ip,
                            has_gateway: has_gw,
                            metric: a.Ipv4Metric,
                            public,
                        });
                    }
                    u = ua.Next;
                }
                p = a.Next;
            }
        }
        out
    }
}

#[cfg(not(windows))]
mod sys {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;

    /// /proc/net/route: interface -> lowest metric of its default routes (destination and mask 0).
    pub fn default_routes(text: &str) -> HashMap<String, u32> {
        let mut out: HashMap<String, u32> = HashMap::new();
        for l in text.lines().skip(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() >= 8 && f[1] == "00000000" && f[7] == "00000000" {
                let metric = f[6].parse().unwrap_or(0);
                let m = out.entry(f[0].to_string()).or_insert(metric);
                *m = (*m).min(metric);
            }
        }
        out
    }

    /// (IfType, description) for `classify`, which reads the description the way it reads a Windows adapter's: what kind of
    /// link the interface is, from its name and sysfs.
    fn kind(name: &str, loopback: bool) -> (u32, &'static str) {
        let sys = Path::new("/sys/class/net").join(name);
        let ty: u32 = std::fs::read_to_string(sys.join("type")).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(1);
        let driver = std::fs::read_link(sys.join("device").join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
        if loopback || ty == 772 {
            (IF_LOOPBACK, "loopback")
        } else if name.starts_with("tailscale") {
            (IF_TUNNEL, "Tailscale")
        } else if name.starts_with("zt") {
            (6, "ZeroTier")
        } else if driver == "rndis_host" || driver == "ipheth" {
            (6, "USB-tethered phone (rndis)")
        } else if ty == 512 {
            (IF_PPP, "PPP link")
        } else if sys.join("tun_flags").exists() || matches!(ty, 65534 | 768 | 769 | 776 | 778 | 823) {
            // tun/tap, WireGuard (no link-layer header), IP-in-IP, sit, GRE
            (IF_TUNNEL, "VPN or tunnel")
        } else if sys.join("wireless").exists() || sys.join("phy80211").exists() {
            (IF_WIFI, "Wi-Fi")
        } else if std::fs::canonicalize(&sys).map(|p| p.to_string_lossy().contains("/virtual/")).unwrap_or(false) {
            // bridges and veth pairs of Docker, Podman, libvirt (virbr0), LXC
            (6, "virtual bridge (pseudo-interface)")
        } else {
            (6, "Ethernet")
        }
    }

    /// The IPv4 addresses of this PC's interfaces (getifaddrs). Linux has no Public/Private network profiles: `public` is None.
    pub fn adapters() -> Vec<Adapter> {
        let routes = default_routes(&std::fs::read_to_string("/proc/net/route").unwrap_or_default());
        let mut out = Vec::new();
        // SAFETY: getifaddrs hands out a list that stays valid until freeifaddrs; only AF_INET entries are read as sockaddr_in
        unsafe {
            let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
            if libc::getifaddrs(&mut list) != 0 {
                return out;
            }
            let mut p = list;
            while !p.is_null() {
                let a = &*p;
                p = a.ifa_next;
                if a.ifa_addr.is_null() || (*a.ifa_addr).sa_family as i32 != libc::AF_INET {
                    continue;
                }
                let ip = Ipv4Addr::from(u32::from_be((*(a.ifa_addr as *const libc::sockaddr_in)).sin_addr.s_addr));
                let name = std::ffi::CStr::from_ptr(a.ifa_name).to_string_lossy().into_owned();
                let flags = a.ifa_flags as i32;
                let (if_type, description) = kind(&name, flags & libc::IFF_LOOPBACK != 0);
                let route = routes.get(&name).copied();
                out.push(Adapter {
                    up: flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0,
                    friendly_name: name,
                    description: description.into(),
                    if_type,
                    ip,
                    has_gateway: route.is_some(),
                    metric: route.unwrap_or(0),
                    public: None,
                });
            }
            libc::freeifaddrs(list);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(name: &str, desc: &str, ip: &str) -> Adapter {
        Adapter { friendly_name: name.into(), description: desc.into(), if_type: 6, up: true, ip: ip.parse().unwrap(), has_gateway: false, metric: 50, public: Some(false) }
    }
    fn gw(mut a: Adapter, metric: u32) -> Adapter {
        a.has_gateway = true;
        a.metric = metric;
        a
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_routes_and_interfaces() {
        let route = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                     enp3s0\t00000000\t0100000A\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                     wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                     wlan0\t00000000\t0101A8C0\t0003\t0\t0\t20\t00000000\t0\t0\t0\n\
                     enp3s0\t0000000A\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        let r = super::sys::default_routes(route);
        assert_eq!((r.get("enp3s0"), r.get("wlan0"), r.len()), (Some(&100), Some(&20), 2));
        // what sys::adapters makes of a Bazzite PC: the LAN first, Tailscale as an overlay, the rest not offered
        let lin = |name: &str, desc: &str, ty: u32, ip: &str| Adapter {
            friendly_name: name.into(),
            description: desc.into(),
            if_type: ty,
            up: true,
            ip: ip.parse().unwrap(),
            has_gateway: false,
            metric: 0,
            public: None,
        };
        let v = vec![
            lin("lo", "loopback", IF_LOOPBACK, "127.0.0.1"),
            gw(lin("enp3s0", "Ethernet", 6, "10.0.0.5"), 100),
            lin("tailscale0", "Tailscale", IF_TUNNEL, "100.101.102.103"),
            lin("virbr0", "virtual bridge (pseudo-interface)", 6, "192.168.122.1"),
            lin("wg0", "VPN or tunnel", IF_TUNNEL, "10.8.0.2"),
            lin("usb0", "USB-tethered phone (rndis)", 6, "192.168.42.129"),
        ];
        let rep = rank(&v);
        assert_eq!(rep.lan.iter().map(|a| a.ip.as_str()).collect::<Vec<_>>(), vec!["10.0.0.5"]);
        assert!(rep.lan[0].default_route && !rep.lan[0].public_profile);
        assert_eq!(rep.overlay.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), vec!["internet via Tailscale"]);
        let why = |name: &str| rep.skipped.iter().find(|s| s.adapter == name).map(|s| s.reason.clone());
        assert_eq!(why("virbr0").as_deref(), Some("virtual adapter"));
        assert_eq!(why("wg0").as_deref(), Some("VPN or tunnel adapter"));
        assert_eq!(why("usb0").as_deref(), Some("USB-tethered phone"));
        assert!(rep.warning.is_none() && rep.firewall_hint.is_none());
    }

    #[test]
    fn default_route_first_and_virtual_skipped() {
        let v = vec![
            ad("vEthernet (Default Switch)", "Hyper-V Virtual Ethernet Adapter", "172.28.0.1"),
            ad("vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter #2", "172.29.0.1"),
            ad("VirtualBox Host-Only Network", "VirtualBox Host-Only Ethernet Adapter", "10.56.0.1"),
            ad("VMware Network Adapter VMnet8", "VMware Virtual Ethernet Adapter for VMnet8", "10.100.0.1"),
            ad("Ethernet 2", "Realtek PCIe GbE", "10.0.0.7"),
            gw(ad("Ethernet", "Intel I225-V", "10.0.0.5"), 25),
            ad("Loopback", "Software Loopback Interface 1", "127.0.0.1"),
            ad("Ethernet 3", "Broken", "169.254.12.3"),
        ];
        let r = rank(&v);
        assert_eq!(r.lan.iter().map(|a| a.ip.as_str()).collect::<Vec<_>>(), ["10.0.0.5", "10.0.0.7"]);
        assert!(r.lan[0].default_route && !r.lan[1].default_route);
        assert_eq!(r.skipped.len(), 6);
    }

    #[test]
    fn usb_phone_hotspot_never_offered() {
        let mut phone = gw(ad("Ethernet 4", "Apple Mobile Device Ethernet", "172.20.10.2"), 5); // best metric, still skipped
        phone.public = Some(true);
        let rndis = gw(ad("Ethernet 5", "Remote NDIS based Internet Sharing Device", "10.42.0.129"), 5);
        let by_range = ad("Ethernet 6", "USB 10/100 LAN", "172.20.10.3");
        let wifi = gw(ad("Wi-Fi", "Intel AX211", "172.16.1.20"), 40);
        let r = rank(&[phone, rndis, by_range, wifi]);
        assert_eq!(r.lan.len(), 1);
        assert_eq!(r.lan[0].ip, "172.16.1.20");
        assert!(r.skipped.iter().all(|s| s.reason == "USB-tethered phone"));
        assert_eq!(r.skipped.len(), 3);
        assert!(r.warning.is_none());
    }

    #[test]
    fn wifi_in_iphone_hotspot_range_is_kept() {
        let mut w = gw(ad("Wi-Fi", "Intel AX211", "172.20.10.4"), 40);
        w.if_type = IF_WIFI;
        assert_eq!(rank(&[w]).lan.len(), 1);
    }

    #[test]
    fn tailscale_and_zerotier_listed_separately() {
        let v = vec![
            gw(ad("Ethernet", "Intel I225-V", "172.16.0.9"), 25),
            ad("Tailscale", "Tailscale Tunnel", "100.101.102.103"),
            ad("ZeroTier One [abc]", "ZeroTier Virtual Port", "10.147.17.5"),
            ad("OpenVPN TAP", "TAP-Windows Adapter V9", "10.8.0.2"),
            ad("Wintun", "WireGuard Tunnel", "10.9.0.2"),
        ];
        let r = rank(&v);
        assert_eq!(r.lan.len(), 1);
        assert_eq!(r.overlay.len(), 2);
        assert_eq!(r.overlay[0].label, "internet via Tailscale");
        assert_eq!(r.overlay[1].label, "internet via ZeroTier");
        assert_eq!(r.skipped.len(), 2);
    }

    #[test]
    fn lowest_metric_gateway_wins_and_private_before_other() {
        let v = vec![
            gw(ad("Wi-Fi", "Intel AX211", "172.16.1.20"), 40),
            gw(ad("Ethernet", "Intel I225-V", "10.1.1.5"), 25),
            ad("Other", "USB NIC", "100.70.0.2"), // CGNAT, not Tailscale
        ];
        let r = rank(&v);
        assert_eq!(r.lan.iter().map(|a| a.ip.as_str()).collect::<Vec<_>>(), ["10.1.1.5", "172.16.1.20", "100.70.0.2"]);
        assert!(r.lan[0].default_route);
        assert!(!r.lan[1].default_route);
    }

    #[test]
    fn public_profile_on_best_address_warns() {
        let mut e = gw(ad("Ethernet", "Intel I225-V", "172.16.0.9"), 25);
        e.public = Some(true);
        let r = rank(&[e]);
        assert_eq!(r.warning.as_deref(), Some(PUBLIC_WARNING));
        assert!(r.firewall_hint.is_some());
        assert!(r.lan[0].public_profile);
        // Private default route plus a Public secondary: no warning
        let mut s = ad("Wi-Fi", "Intel AX211", "172.16.1.20");
        s.public = Some(true);
        let r = rank(&[gw(ad("Ethernet", "x", "10.0.0.5"), 10), s]);
        assert!(r.warning.is_none() && r.firewall_hint.is_none());
        assert!(r.lan[1].public_profile);
    }

    #[test]
    fn unknown_profile_does_not_warn_and_down_adapters_ignored() {
        let mut a = gw(ad("Ethernet", "x", "10.0.0.5"), 10);
        a.public = None;
        let mut d = ad("Wi-Fi", "y", "10.0.0.6");
        d.up = false;
        let r = rank(&[a, d]);
        assert!(r.warning.is_none());
        assert_eq!(r.lan.len(), 1);
        assert!(r.skipped.is_empty());
    }

    #[test]
    fn empty_list_gives_hint() {
        let r = rank(&[]);
        assert!(r.lan.is_empty() && r.firewall_hint.is_some());
        assert!(log_lines(&r, 39660)[0].starts_with("No usable"));
    }
}
