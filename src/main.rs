mod cli;
mod commands;
mod conflict;
mod detach;
mod expire;
mod fetch;
mod fsx;
mod graph;
mod heartbeat;
mod help;
mod holder;
mod home;
mod i18n;
mod index;
mod key;
mod lease;
mod log;
mod oslock;
mod queue;
mod selfreplace;
mod sha256;
mod store;
mod sweep;
mod wait;

mod cmd {
    pub mod about;
    pub mod acquire;
    pub mod doctor;
    pub mod gc;
    pub mod graph;
    pub mod heartbeat;
    pub mod hold;
    pub mod list;
    pub mod release;
    pub mod renew;
    pub mod run;
    pub mod status;
    pub mod unlock;
    pub mod update;
}

fn main() {
    #[cfg(windows)]
    selfreplace::cleanup_old();
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::main(args));
}
