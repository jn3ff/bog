use clap::Parser;

fn main() {
    let cli = bog::cli::Cli::parse();
    if let Err(e) = bog::cli::run(cli) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
