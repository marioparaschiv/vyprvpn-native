use std::net::Ipv4Addr;

/// `netsh` argument lists that set the adapter's DNS servers, in order of preference.
pub(crate) fn dns_commands(adapter: &str, servers: &[Ipv4Addr]) -> Vec<Vec<String>> {
    let name = format!("name={adapter}");

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

            args.into_iter().map(str::to_owned).collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_the_first_server_and_appends_the_rest() {
        let commands = dns_commands(
            "VyprLite",
            &[Ipv4Addr::new(10, 21, 0, 1), Ipv4Addr::new(10, 21, 0, 2)],
        );

        assert_eq!(
            commands,
            [
                vec![
                    "interface",
                    "ipv4",
                    "set",
                    "dnsservers",
                    "name=VyprLite",
                    "source=static",
                    "address=10.21.0.1",
                    "register=none",
                    "validate=no",
                ],
                vec![
                    "interface",
                    "ipv4",
                    "add",
                    "dnsservers",
                    "name=VyprLite",
                    "address=10.21.0.2",
                    "index=2",
                    "validate=no",
                ],
            ]
        );
    }

    #[test]
    fn no_servers_means_no_commands() {
        assert!(dns_commands("VyprLite", &[]).is_empty());
    }
}
