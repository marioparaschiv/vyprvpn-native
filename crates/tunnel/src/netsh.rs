use std::net::Ipv4Addr;
use std::path::Path;

use crate::command::Invocation;

/// `netsh` invocations that set the adapter's DNS servers, in order of preference. With no
/// servers they clear the static ones, which would otherwise survive from an earlier session on
/// the same adapter.
pub(crate) fn dns_commands(netsh: &Path, adapter: &str, servers: &[Ipv4Addr]) -> Vec<Invocation> {
    let name = format!("name={adapter}");

    if servers.is_empty() {
        return vec![Invocation::new(
            netsh,
            &[
                "interface",
                "ipv4",
                "set",
                "dnsservers",
                &name,
                "source=static",
                "address=none",
                "validate=no",
            ],
        )];
    }

    servers
        .iter()
        .enumerate()
        .map(|(position, server)| {
            let address = format!("address={server}");
            let index = format!("index={}", position + 1);
            let args: Vec<&str> = match position {
                0 => vec![
                    "interface",
                    "ipv4",
                    "set",
                    "dnsservers",
                    &name,
                    "source=static",
                    &address,
                    "register=none",
                    "validate=no",
                ],
                _ => vec![
                    "interface",
                    "ipv4",
                    "add",
                    "dnsservers",
                    &name,
                    &address,
                    &index,
                    "validate=no",
                ],
            };

            Invocation::new(netsh, &args)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETSH: &str = r"C:\Windows\System32\netsh.exe";

    #[test]
    fn sets_the_first_server_and_appends_the_rest() {
        let commands = dns_commands(
            Path::new(NETSH),
            "VyprLite",
            &[Ipv4Addr::new(10, 21, 0, 1), Ipv4Addr::new(10, 21, 0, 2)],
        );

        assert_eq!(
            commands,
            [
                Invocation::new(
                    NETSH,
                    &[
                        "interface",
                        "ipv4",
                        "set",
                        "dnsservers",
                        "name=VyprLite",
                        "source=static",
                        "address=10.21.0.1",
                        "register=none",
                        "validate=no",
                    ]
                ),
                Invocation::new(
                    NETSH,
                    &[
                        "interface",
                        "ipv4",
                        "add",
                        "dnsservers",
                        "name=VyprLite",
                        "address=10.21.0.2",
                        "index=2",
                        "validate=no",
                    ]
                ),
            ]
        );
    }

    #[test]
    fn no_servers_clears_the_static_ones() {
        assert_eq!(
            dns_commands(Path::new(NETSH), "VyprLite", &[]),
            [Invocation::new(
                NETSH,
                &[
                    "interface",
                    "ipv4",
                    "set",
                    "dnsservers",
                    "name=VyprLite",
                    "source=static",
                    "address=none",
                    "validate=no",
                ]
            )]
        );
    }
}
