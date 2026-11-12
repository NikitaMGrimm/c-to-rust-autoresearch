use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: transpiler <input.c> [-o <output.rs>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = match args.as_slice() {
        [input] => (input, None),
        [input, flag, output] if flag == "-o" => (input, Some(output)),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Some(output) = output
        && same_file(Path::new(input), Path::new(output))
    {
        eprintln!("refusing to overwrite the input file {input}");
        return ExitCode::FAILURE;
    }
    let rust = match transpiler::translate_file(Path::new(input)) {
        Ok(rust) => rust,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    match output {
        Some(path) => {
            if let Err(err) = std::fs::write(path, rust) {
                eprintln!("cannot write {path}: {err}");
                return ExitCode::FAILURE;
            }
        }
        None => print!("{rust}"),
    }
    ExitCode::SUCCESS
}

/// Compares device and inode, so symlinks and hard links to the input match.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}
