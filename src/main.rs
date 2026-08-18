use std::collections::HashSet;
use std::env;
use std::net::Ipv4Addr;

use localhost::config::load_config;

use localhost::net::listener::bind_listener;

use localhost::server::event_loop::EventLoop;

fn main() {
    let config_path =
        env::args()
            .nth(1)
            .unwrap_or_else(|| {
                "config.toml".to_string()
            });

    let config =
        match load_config(&config_path) {
            Ok(config) => config,

            Err(err) => {
                eprintln!(
                    "configuration error: {}",
                    err
                );

                std::process::exit(1);
            }
        };

    let mut listeners = Vec::new();

    let mut bound =
        HashSet::<(Ipv4Addr, u16)>::new();

    for server in &config.servers {
        let address =
            match server
                .server_address
                .parse::<Ipv4Addr>()
            {
                Ok(address) => address,

                Err(err) => {
                    eprintln!(
                        "invalid address '{}': {}",
                        server.server_address,
                        err
                    );

                    continue;
                }
            };

        for &port in &server.ports {
            /*
             * Multiple virtual servers are allowed to
             * share the same address and port.
             *
             * Only one actual listening socket is
             * required. Phase 6 will choose the
             * correct virtual server using Host.
             */
            if !bound.insert((address, port)) {
                continue;
            }

            match bind_listener(
                address,
                port,
            ) {
                Ok(listener) => {
                    println!(
                        "listening on {}:{}",
                        address,
                        listener.port
                    );

                    listeners.push(listener);
                }

                Err(err) => {
                    eprintln!(
                        "failed to bind {}:{}: {}",
                        address,
                        port,
                        err
                    );
                }
            }
        }
    }

    if listeners.is_empty() {
        eprintln!(
            "no listeners could be created"
        );

        std::process::exit(1);
    }

    let mut event_loop =
        match EventLoop::new(listeners) {
            Ok(event_loop) => event_loop,

            Err(err) => {
                eprintln!(
                    "failed to initialize event loop: {}",
                    err
                );

                std::process::exit(1);
            }
        };

    if let Err(err) =
        event_loop.run()
    {
        eprintln!(
            "event loop terminated: {}",
            err
        );

        std::process::exit(1);
    }
}