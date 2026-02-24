use clap::Parser;

fn main() {
    let cli = bogbot::cli::Cli::parse();
    if let Err(e) = bogbot::cli::run(cli) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
