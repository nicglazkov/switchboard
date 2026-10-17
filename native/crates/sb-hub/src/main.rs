//! switchboard-hub: see lib.rs. Settings come from the environment (config.rs); `--listen ADDR` overrides one.

use sb_hub::config::Config;
use sb_hub::server::{self, Hub, log};

fn usage() -> ! {
    eprintln!("usage: switchboard-hub [--listen ADDR] [--version]   (other settings: SWITCHBOARD_* variables, see config.rs)");
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
    let mut cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("switchboard-hub: {e}");
            std::process::exit(2);
        }
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => cfg.listen = args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage()),
            "--version" => {
                println!("switchboard-hub {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => usage(),
        }
    }
    let listen = cfg.listen;
    let hub = match Hub::new(cfg) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("switchboard-hub: ticket key: {e}");
            std::process::exit(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind(listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("switchboard-hub: cannot listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    log(format!("switchboard-hub {} on {listen}, up to {} terminals", env!("CARGO_PKG_VERSION"), hub.cfg.max_terms));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
    tokio::select! {
        r = server::serve(hub, listener) => if let Err(e) = r {
            eprintln!("switchboard-hub: {e}");
            std::process::exit(1);
        },
        _ = term.recv() => log("switchboard-hub stopping; open terminals detach"),
        _ = tokio::signal::ctrl_c() => log("switchboard-hub stopping; open terminals detach"),
    }
}
