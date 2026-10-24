// Helpers land before the commands that use them; this goes away once every command exists.
#![allow(dead_code)]

mod cli;
mod conflict;
mod expire;
mod fsx;
mod graph;
mod heartbeat;
mod holder;
mod home;
mod index;
mod key;
mod log;
mod oslock;
mod queue;
mod store;
mod sweep;
mod wait;

mod cmd {
    pub mod acquire;
    pub mod gc;
    pub mod graph;
    pub mod heartbeat;
    pub mod hold;
    pub mod list;
    pub mod release;
    pub mod run;
    pub mod status;
    pub mod unlock;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::main(args));
}
