use std::sync::Arc;

fn main() {
    let repo = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |key: &str| std::env::var(key).unwrap_or_default();
    std::process::exit(harness::cli::run(
        &args,
        &mut std::io::stdout(),
        None,
        &repo,
        Arc::new(harness::tools::Exec),
        &env,
    ));
}
