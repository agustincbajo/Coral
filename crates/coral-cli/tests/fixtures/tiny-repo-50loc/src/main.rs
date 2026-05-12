//! Tiny greeter CLI. Prints `greet("world")` or `greet(argv[1])`.

use tiny_greeter::greet;

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "world".to_string());
    println!("{}", greet(&name));
}
