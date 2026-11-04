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
    let ast = match transpiler::clang_ast(Path::new(input)) {
        Ok(ast) => ast,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let rust = transpiler::translate(&ast);
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
