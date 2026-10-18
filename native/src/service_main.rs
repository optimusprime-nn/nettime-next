fn main() {
    if let Err(e) = nettime_next_native::run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
