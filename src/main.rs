use std::net::Ipv4Addr;

use localhost::net::listener::bind_listener;
use localhost::server::event_loop::EventLoop;

/// Phase 2 entry point: no configuration file yet (that's Phase 3). Binds a
/// small fixed set of listeners purely to exercise the networking
/// foundation -- multiple listening sockets feeding the same single event
/// loop.
fn main() {
    let ports: [u16; 2] = [8080, 8081];
    let mut listeners = Vec::new();

    for &port in &ports {
        match bind_listener(Ipv4Addr::UNSPECIFIED, port) {
            Ok(listener) => {
                println!("listening on 0.0.0.0:{}", port);
                listeners.push(listener);
            }
            Err(err) => {
                eprintln!("failed to bind port {}: {}", port, err);
            }
        }
    }

    if listeners.is_empty() {
        eprintln!("no listeners bound, exiting");
        std::process::exit(1);
    }

    let mut event_loop = match EventLoop::new(listeners) {
        Ok(ev) => ev,
        Err(err) => {
            eprintln!("failed to initialize event loop: {}", err);
            std::process::exit(1);
        }
    };

    if let Err(err) = event_loop.run() {
        eprintln!("event loop terminated: {}", err);
        std::process::exit(1);
    }
}
