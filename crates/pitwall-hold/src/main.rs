//! `pitwall-hold` binary; see the library docs for usage and the protocol.

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().and_then(|a| a.to_str()) == Some("--version") {
        println!("pitwall-hold {} protocol {}", env!("CARGO_PKG_VERSION"), pitwall_hold::PROTOCOL_VERSION);
        return;
    }
    let code = match pitwall_hold::server::parse_args(args) {
        Ok(cfg) => pitwall_hold::server::run(cfg),
        Err(e) => {
            eprintln!("pitwall-hold: {e}");
            2
        }
    };
    std::process::exit(code);
}
