//! switchboard-hub: see lib.rs. Settings come from the environment (config.rs); `--listen ADDR` and
//! `--tunnel-listen ADDR` override two. "keys" comes from the server's server.json (config.rs `read_keys`).

use sb_hub::config::{self, Config, Keys};
use sb_hub::server::{self, Hub, log};
use std::sync::Arc;
use std::time::Duration;

fn usage() -> ! {
    eprintln!(
        "usage: switchboard-hub [--listen ADDR] [--tunnel-listen ADDR] [--version]   (other settings: SWITCHBOARD_* \
         variables, see config.rs)"
    );
    std::process::exit(2);
}

/// The tunnel listener, for as long as the hub runs. A port that is taken (another hub still stopping) is tried
/// again; the main listener is not held up by it.
async fn tunnel(hub: Arc<Hub>, addr: std::net::SocketAddr) {
    let mut said = false;
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => {
                log(format!("switchboard-hub: tunnel connections on {addr}"));
                said = false;
                if let Err(e) = server::serve_public(hub.clone(), l).await {
                    log(format!("switchboard-hub: the tunnel listener stopped: {e}"));
                }
            }
            Err(e) if !said => {
                log(format!("switchboard-hub: cannot listen on {addr} for tunnel connections yet: {e}"));
                said = true;
            }
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
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
            "--tunnel-listen" => {
                cfg.tunnel_listen = match args.next().map(|v| config::tunnel_addr(&v)) {
                    Some(Ok(a)) => a,
                    Some(Err(e)) => {
                        eprintln!("switchboard-hub: --tunnel-listen: {e}");
                        std::process::exit(2);
                    }
                    None => usage(),
                }
            }
            "--version" => {
                println!("switchboard-hub {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => usage(),
        }
    }
    if let Some(note) = cfg.load_keys() {
        log(format!("switchboard-hub: {note}"));
    }
    // A computer that stopped being the home computer keeps a note next to the server's settings (sb-node
    // home.rs, server/homestate.py). While it is there this hub answers nobody: the hub that counts is on the
    // computer that is home now. It waits instead of ending, so that whatever starts it does not start it again
    // and again.
    let note = cfg.server_json.with_file_name("home-stepdown.json");
    if note.exists() {
        log("switchboard-hub: This computer was the home computer until it moved. It is not answering as home.");
        while note.exists() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        let _ = cfg.load_keys();
    }
    let (asked, listen, tunnel_at) = (cfg.listen, cfg.main_listen(), cfg.tunnel());
    if listen != asked {
        log(format!("switchboard-hub: keys only, so it listens on {listen} and not on {asked}"));
    }
    let keys = cfg.keys;
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
    let mode = match keys {
        Keys::Off => "",
        Keys::Both => ", keys on",
        Keys::Only => ", keys only",
    };
    log(format!(
        "switchboard-hub {} on {listen}{mode}, up to {} terminals",
        env!("CARGO_PKG_VERSION"),
        hub.cfg.max_terms
    ));
    if let Some(addr) = tunnel_at {
        tokio::spawn(tunnel(hub.clone(), addr));
    }
    // SIGTERM is how a service manager stops it; Windows has no such signal, only Ctrl+C.
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
    #[cfg(unix)]
    let terminate = term.recv();
    #[cfg(not(unix))]
    let terminate = std::future::pending::<Option<()>>();
    tokio::select! {
        r = server::serve(hub, listener) => if let Err(e) = r {
            eprintln!("switchboard-hub: {e}");
            std::process::exit(1);
        },
        _ = terminate => log("switchboard-hub stopping; open terminals detach"),
        _ = tokio::signal::ctrl_c() => log("switchboard-hub stopping; open terminals detach"),
    }
}
