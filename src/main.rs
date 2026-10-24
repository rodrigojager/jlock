// Helpers land before the commands that use them; this goes away once every command exists.
#![allow(dead_code)]

mod cli;
mod fsx;
mod holder;
mod home;
mod key;
mod store;

mod cmd {
    pub mod acquire;
    pub mod release;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::main(args));
}
