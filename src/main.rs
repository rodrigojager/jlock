// Helpers land before the commands that use them; this goes away once every command exists.
#![allow(dead_code)]

mod cli;
mod expire;
mod fsx;
mod heartbeat;
mod holder;
mod home;
mod key;
mod queue;
mod store;
mod wait;

mod cmd {
    pub mod acquire;
    pub mod heartbeat;
    pub mod hold;
    pub mod release;
    pub mod run;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::main(args));
}
