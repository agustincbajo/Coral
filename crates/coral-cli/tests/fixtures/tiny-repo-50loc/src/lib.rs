//! Tiny greeter library.
//!
//! Two functions. Both pure, both unit-tested. Total ~30 LOC including
//! tests so a local-LLM bootstrap finishes inside the 10-minute test
//! budget without GPU acceleration.

/// Returns `"Hello, <name>!"`.
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

/// Returns the greeting in upper case with three exclamation marks.
pub fn shout(name: &str) -> String {
    format!("HELLO, {}!!!", name.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_with_name() {
        assert_eq!(greet("world"), "Hello, world!");
    }

    #[test]
    fn shout_uppercases_and_emphasizes() {
        assert_eq!(shout("ada"), "HELLO, ADA!!!");
    }
}
