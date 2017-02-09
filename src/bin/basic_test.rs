extern crate wow_combat_log;

use std::fs::File;
use std::io::BufReader;

fn main() {
    let read = BufReader::new(File::open(std::env::args().nth(1).unwrap()).unwrap());
    let intern = wow_combat_log::Interner::default();
    for log in wow_combat_log::iter(&intern, read) {
        println!("{:?}", log);
    }
}
