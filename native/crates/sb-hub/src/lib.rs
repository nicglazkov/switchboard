//! Switchboard hub (spec section 1): terminals for the web page, the phone apps and the desktop app.
//! Each terminal is a PTY on the hub running `ssh -tt <machine> term <bg_id>`, relayed over a WebSocket. The
//! machine side (`sb.py`) turns `term <bg_id>` into `claude attach <bg_id>`; closing only detaches.
//! A machine paired with switchboard-node (spec section 6) links out to `/node` instead, and its RPCs and
//! terminals go over that link, so it needs no SSH.

pub mod config;
pub mod link;
pub mod link_proto;
pub mod machines;
pub mod nodeapi;
pub mod nodes;
pub mod origin;
pub mod proto;
pub mod pty;
pub mod server;
pub mod shared;
pub mod ticket;
