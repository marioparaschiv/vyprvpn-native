use std::collections::BTreeSet;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::plan::AdapterPlan;

pub(crate) const IFCONFIG: &str = "/sbin/ifconfig";
pub(crate) const ROUTE: &str = "/sbin/route";
pub(crate) const NETWORKSETUP: &str = "/usr/sbin/networksetup";
const MTU: u16 = 1420;
const NO_DNS_SERVERS: &str = "There aren't any DNS Servers set on";

/// One command to run, with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub program: &'static str,
    pub args: Vec<String>,
}

impl Invocation {
    pub(crate) fn new(program: &'static str, args: &[&str]) -> Self {
        Self {
            program,
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
        }
    }
}

impl fmt::Display for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.program, self.args.join(" "))
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("no IPv4 gateway in `route -n get default` output: {0:?}")]
    NoGateway(String),
    #[error("no interface in `route -n get default` output: {0:?}")]
    NoInterface(String),
    #[error("no enabled network service uses device {0}")]
    NoService(String),
    #[error("invalid DNS server {0:?} in networksetup output")]
    Dns(String),
    #[error("no errno in the WireGuard UAPI reply {0:?}")]
    UapiReply(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DefaultRoute {
    pub gateway: Ipv4Addr,
    pub interface: String,
}

/// Reads the `gateway` and `interface` fields of `route -n get default`.
pub(crate) fn parse_default_route(output: &str) -> Result<DefaultRoute, ParseError> {
    let field = |name: &str| {
        output.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == name).then(|| value.trim())
        })
    };
    let gateway = field("gateway")
        .and_then(|gateway| gateway.parse().ok())
        .ok_or_else(|| ParseError::NoGateway(output.to_owned()))?;
    let interface = field("interface")
        .filter(|interface| !interface.is_empty())
        .ok_or_else(|| ParseError::NoInterface(output.to_owned()))?;

    Ok(DefaultRoute {
        gateway,
        interface: interface.to_owned(),
    })
}

/// Finds the enabled service for `device` in `networksetup -listnetworkserviceorder`, where each
/// service is a `(1) Name` line (`(*) Name` when disabled) followed by its hardware port line.
pub(crate) fn service_for_device(output: &str, device: &str) -> Result<String, ParseError> {
    let port_suffix = format!(", Device: {device})");
    let mut service = None;

    for line in output.lines() {
        if line.starts_with("(Hardware Port: ") {
            if line.ends_with(&port_suffix)
                && let Some(name) = service.take()
            {
                return Ok(name);
            }
            continue;
        }

        service = line
            .strip_prefix('(')
            .and_then(|rest| rest.split_once(") "))
            .filter(|(marker, _)| marker.parse::<u32>().is_ok())
            .map(|(_, name)| name.to_owned());
    }

    Err(ParseError::NoService(device.to_owned()))
}

/// Parses `networksetup -getdnsservers`, which prints one server per line or a sentence when
/// the service has none of its own.
pub(crate) fn parse_dns_servers(output: &str) -> Result<Vec<IpAddr>, ParseError> {
    if output.trim_start().starts_with(NO_DNS_SERVERS) {
        return Ok(Vec::new());
    }

    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.parse().map_err(|_| ParseError::Dns(line.to_owned())))
        .collect()
}

/// The lowest `utunN` absent from `ifconfig -l`.
pub(crate) fn free_utun(interfaces: &str) -> String {
    let taken: BTreeSet<u32> = interfaces
        .split_whitespace()
        .filter_map(|name| name.strip_prefix("utun")?.parse().ok())
        .collect();
    let mut free = 0;
    for index in taken {
        if index != free {
            break;
        }
        free += 1;
    }

    format!("utun{free}")
}

/// Where boringtun serves the WireGuard UAPI for `interface`.
pub(crate) fn uapi_socket(interface: &str) -> String {
    format!("/var/run/wireguard/{interface}.sock")
}

/// The UAPI `set` request that gives the interface its key and its single peer.
pub(crate) fn uapi_set(plan: &AdapterPlan) -> Zeroizing<String> {
    let peer = &plan.peer;
    let mut text = Zeroizing::new(String::from("set=1\nprivate_key="));
    push_hex(&mut text, plan.private_key.as_bytes());
    text.push_str("\nreplace_peers=true\npublic_key=");
    push_hex(&mut text, peer.public_key.as_bytes());
    text.push_str("\npreshared_key=");
    push_hex(&mut text, peer.preshared_key.as_bytes());
    text.push_str(&format!(
        "\nendpoint={}\nreplace_allowed_ips=true\n",
        peer.endpoint
    ));
    for net in peer.allowed_ips {
        text.push_str(&format!("allowed_ip={net}\n"));
    }
    text.push('\n');

    text
}

/// Writes digits straight into `text` so no unzeroized copy of a secret is made.
fn push_hex(text: &mut String, bytes: &[u8; 32]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0xf)]));
    }
}

/// The `errno` of a UAPI reply such as `errno=0\n\n`.
pub(crate) fn parse_uapi_reply(reply: &str) -> Result<i32, ParseError> {
    reply
        .lines()
        .find_map(|line| line.strip_prefix("errno=")?.parse().ok())
        .ok_or_else(|| ParseError::UapiReply(reply.to_owned()))
}

/// Whether a failed `route delete` failed only because the route was already gone.
pub(crate) fn route_was_missing(output: &str) -> bool {
    output.contains("not in table")
}

/// Whether networksetup reported an error, which it can do while still exiting 0.
pub(crate) fn networksetup_failed(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.trim_start().starts_with("** Error"))
}

/// A service's DNS servers before VyprLite replaced them. No servers means the service had none
/// of its own and used the ones from DHCP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedDns {
    pub(crate) service: String,
    pub(crate) servers: Vec<IpAddr>,
}

/// What undoes a tunnel's changes to the host. Plain data, so the helper can store it once the
/// tunnel is up and replay it with `Teardown::run` after a crash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Teardown {
    pub(crate) endpoint: Ipv4Addr,
    pub(crate) gateway: Ipv4Addr,
    pub(crate) dns: Option<SavedDns>,
}

impl Teardown {
    /// The split routes are not removed here: the kernel drops them along with the utun, which
    /// closes with the process that owns it.
    pub(crate) fn commands(&self) -> Vec<Invocation> {
        let mut commands = Vec::new();

        if let Some(SavedDns { service, servers }) = &self.dns {
            let mut args = vec!["-setdnsservers".to_owned(), service.clone()];
            match servers.as_slice() {
                [] => args.push("Empty".to_owned()),
                servers => args.extend(servers.iter().map(IpAddr::to_string)),
            }
            commands.push(Invocation {
                program: NETWORKSETUP,
                args,
            });
        }

        commands.push(Invocation::new(
            ROUTE,
            &["-n", "delete", "-inet", &self.endpoint.to_string()],
        ));

        commands
    }
}

/// Commands that configure `interface` for `plan` once boringtun has created it: address and
/// MTU, a route keeping the endpoint on the original gateway, the split default route, then DNS
/// for the service `teardown` saved it from.
pub(crate) fn up_commands(
    interface: &str,
    plan: &AdapterPlan,
    teardown: &Teardown,
) -> Vec<Invocation> {
    let address = plan.address.addr().to_string();
    let mut commands = vec![
        Invocation::new(
            IFCONFIG,
            &[
                interface,
                "inet",
                &plan.address.to_string(),
                &address,
                "alias",
            ],
        ),
        Invocation::new(IFCONFIG, &[interface, "mtu", &MTU.to_string()]),
        Invocation::new(IFCONFIG, &[interface, "up"]),
        Invocation::new(
            ROUTE,
            &[
                "-n",
                "add",
                "-inet",
                &teardown.endpoint.to_string(),
                "-gateway",
                &teardown.gateway.to_string(),
            ],
        ),
    ];

    commands.extend(plan.peer.allowed_ips.iter().map(|net| {
        Invocation::new(
            ROUTE,
            &[
                "-n",
                "add",
                "-inet",
                &net.to_string(),
                "-interface",
                interface,
            ],
        )
    }));

    if let Some(saved) = &teardown.dns {
        let mut args = vec!["-setdnsservers".to_owned(), saved.service.clone()];
        args.extend(plan.dns.iter().map(Ipv4Addr::to_string));
        commands.push(Invocation {
            program: NETWORKSETUP,
            args,
        });
    }

    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::realistic_plan;

    const ROUTE_GET_DEFAULT: &str = "   route to: default
destination: default
       mask: default
    gateway: 192.168.1.1
  interface: en0
      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING,GLOBAL>
 recvpipe  sendpipe  ssthresh  rtt,msec    rttvar  hopcount      mtu     expire
       0         0         0         0         0         0      1500         0
";

    const SERVICE_ORDER: &str = "An asterisk (*) denotes that a network service is disabled.
(1) USB 10/100/1000 LAN
(Hardware Port: USB 10/100/1000 LAN, Device: en7)

(2) Wi-Fi
(Hardware Port: Wi-Fi, Device: en0)

(3) Thunderbolt Bridge
(Hardware Port: Thunderbolt Bridge, Device: bridge0)

(*) iPhone USB
(Hardware Port: iPhone USB, Device: en8)

";

    const IFCONFIG_LIST: &str = "lo0 gif0 stf0 anpi1 anpi0 en3 en4 en1 en2 bridge0 ap1 en0 \
        awdl0 llw0 utun0 utun1 utun2 utun3\n";

    fn command(program: &'static str, args: &[&str]) -> Invocation {
        Invocation::new(program, args)
    }

    fn teardown(dns: Option<SavedDns>) -> Teardown {
        Teardown {
            endpoint: Ipv4Addr::new(203, 0, 113, 10),
            gateway: Ipv4Addr::new(192, 168, 1, 1),
            dns,
        }
    }

    fn wifi_dns(servers: &[&str]) -> Option<SavedDns> {
        Some(SavedDns {
            service: "Wi-Fi".into(),
            servers: servers
                .iter()
                .map(|server| server.parse().unwrap())
                .collect(),
        })
    }

    #[test]
    fn reads_gateway_and_interface_of_the_default_route() {
        assert_eq!(
            parse_default_route(ROUTE_GET_DEFAULT).unwrap(),
            DefaultRoute {
                gateway: Ipv4Addr::new(192, 168, 1, 1),
                interface: "en0".into(),
            }
        );
    }

    #[test]
    fn a_default_route_without_a_gateway_is_an_error() {
        let output = "   route to: default\ndestination: default\n       mask: default\n  \
            interface: utun4\n      flags: <UP,DONE,STATIC>\n";

        assert!(matches!(
            parse_default_route(output),
            Err(ParseError::NoGateway(_))
        ));
        assert!(matches!(
            parse_default_route("route: writing to routing socket: not in table\n"),
            Err(ParseError::NoGateway(_))
        ));
    }

    #[test]
    fn finds_the_service_of_a_device() {
        assert_eq!(service_for_device(SERVICE_ORDER, "en0").unwrap(), "Wi-Fi");
        assert_eq!(
            service_for_device(SERVICE_ORDER, "en7").unwrap(),
            "USB 10/100/1000 LAN"
        );
    }

    #[test]
    fn disabled_or_unknown_devices_have_no_service() {
        for device in ["en8", "en9", "en"] {
            assert_eq!(
                service_for_device(SERVICE_ORDER, device),
                Err(ParseError::NoService(device.into()))
            );
        }
    }

    #[test]
    fn parses_dns_servers_one_per_line() {
        assert_eq!(
            parse_dns_servers("1.1.1.1\n8.8.8.8\n2606:4700:4700::1111\n").unwrap(),
            [
                IpAddr::from([1, 1, 1, 1]),
                IpAddr::from([8, 8, 8, 8]),
                "2606:4700:4700::1111".parse::<IpAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn a_service_without_its_own_dns_has_no_servers() {
        let output = "There aren't any DNS Servers set on Wi-Fi.\n";

        assert!(parse_dns_servers(output).unwrap().is_empty());
    }

    #[test]
    fn unexpected_dns_output_is_an_error() {
        assert_eq!(
            parse_dns_servers("Wi-Fi is not a recognized network service.\n"),
            Err(ParseError::Dns(
                "Wi-Fi is not a recognized network service.".into()
            ))
        );
    }

    #[test]
    fn picks_the_lowest_free_utun() {
        assert_eq!(free_utun(IFCONFIG_LIST), "utun4");
        assert_eq!(free_utun("lo0 en0 utun0 utun2 utun3"), "utun1");
        assert_eq!(free_utun("lo0 en0"), "utun0");
    }

    #[test]
    fn uapi_socket_is_named_after_the_interface() {
        assert_eq!(uapi_socket("utun4"), "/var/run/wireguard/utun4.sock");
    }

    #[test]
    fn uapi_set_carries_hex_keys_endpoint_and_split_route() {
        let expected = format!(
            "set=1\n\
            private_key={}\n\
            replace_peers=true\n\
            public_key=8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a\n\
            preshared_key=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\n\
            endpoint=203.0.113.10:51820\n\
            replace_allowed_ips=true\n\
            allowed_ip=0.0.0.0/1\n\
            allowed_ip=128.0.0.0/1\n\
            \n",
            "07".repeat(32)
        );

        assert_eq!(uapi_set(&realistic_plan()).as_str(), expected);
    }

    #[test]
    fn reads_the_uapi_errno() {
        assert_eq!(parse_uapi_reply("errno=0\n\n"), Ok(0));
        assert_eq!(parse_uapi_reply("errno=22\n\n"), Ok(22));
        assert_eq!(
            parse_uapi_reply(""),
            Err(ParseError::UapiReply(String::new()))
        );
    }

    #[test]
    fn up_configures_interface_routes_and_dns_in_order() {
        let teardown = teardown(wifi_dns(&["1.1.1.1"]));

        assert_eq!(
            up_commands("utun4", &realistic_plan(), &teardown),
            [
                command(
                    IFCONFIG,
                    &["utun4", "inet", "10.21.0.7/32", "10.21.0.7", "alias"]
                ),
                command(IFCONFIG, &["utun4", "mtu", "1420"]),
                command(IFCONFIG, &["utun4", "up"]),
                command(
                    ROUTE,
                    &[
                        "-n",
                        "add",
                        "-inet",
                        "203.0.113.10",
                        "-gateway",
                        "192.168.1.1"
                    ]
                ),
                command(
                    ROUTE,
                    &["-n", "add", "-inet", "0.0.0.0/1", "-interface", "utun4"]
                ),
                command(
                    ROUTE,
                    &["-n", "add", "-inet", "128.0.0.0/1", "-interface", "utun4"]
                ),
                command(
                    NETWORKSETUP,
                    &["-setdnsservers", "Wi-Fi", "10.21.0.1", "10.21.0.2"]
                ),
            ]
        );
    }

    #[test]
    fn up_leaves_dns_alone_without_a_saved_service() {
        let commands = up_commands("utun4", &realistic_plan(), &teardown(None));

        assert_eq!(commands.len(), 6);
        assert!(
            commands
                .iter()
                .all(|command| command.program != NETWORKSETUP)
        );
    }

    #[test]
    fn teardown_restores_the_saved_servers_then_drops_the_endpoint_route() {
        assert_eq!(
            teardown(wifi_dns(&["1.1.1.1", "2606:4700:4700::1111"])).commands(),
            [
                command(
                    NETWORKSETUP,
                    &["-setdnsservers", "Wi-Fi", "1.1.1.1", "2606:4700:4700::1111"]
                ),
                command(ROUTE, &["-n", "delete", "-inet", "203.0.113.10"]),
            ]
        );
    }

    #[test]
    fn teardown_empties_dns_that_came_from_dhcp() {
        assert_eq!(
            teardown(wifi_dns(&[])).commands()[0],
            command(NETWORKSETUP, &["-setdnsservers", "Wi-Fi", "Empty"])
        );
    }

    #[test]
    fn teardown_without_dns_only_drops_the_endpoint_route() {
        assert_eq!(
            teardown(None).commands(),
            [command(ROUTE, &["-n", "delete", "-inet", "203.0.113.10"])]
        );
    }

    #[test]
    fn teardown_survives_a_round_trip_through_json() {
        let teardown = teardown(wifi_dns(&["1.1.1.1"]));

        let json = serde_json::to_string(&teardown).unwrap();

        assert_eq!(
            json,
            r#"{"endpoint":"203.0.113.10","gateway":"192.168.1.1","dns":{"service":"Wi-Fi","servers":["1.1.1.1"]}}"#
        );
        assert_eq!(serde_json::from_str::<Teardown>(&json).unwrap(), teardown);
    }

    #[test]
    fn recognises_networksetup_errors_printed_with_exit_0() {
        assert!(networksetup_failed(
            "** Error: The parameters were not valid.
"
        ));
        assert!(networksetup_failed(
            "Wi-Fi
** Error: Unable to commit changes to network database.
"
        ));
        assert!(!networksetup_failed(""));
        assert!(!networksetup_failed(
            "1.1.1.1
8.8.8.8
"
        ));
    }

    #[test]
    fn recognises_a_route_that_is_already_gone() {
        assert!(route_was_missing(
            "route: writing to routing socket: not in table\ndelete host 203.0.113.10: not in table\n"
        ));
        assert!(!route_was_missing(
            "route: must be root to alter routing table\n"
        ));
    }
}
