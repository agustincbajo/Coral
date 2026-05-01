//! Print the StatsReport JSON schema to stdout.
//!
//! Usage:
//!   cargo run -p coral-stats --example print_schema > docs/schemas/stats.schema.json
fn main() {
    println!("{}", coral_stats::StatsReport::json_schema());
}
