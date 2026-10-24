//! `phpcraft`: command line entry point.
//!
//! Mirrors the parts of `sapi/cli/php_cli.c` we need: run a file, run code with
//! `-r`, syntax-check with `-l`, and expose `$argv`/`$argc`. PHP's CLI sends
//! diagnostics to stderr and script output to stdout; we do the same.

use std::io::{Read, Write};
use std::process::ExitCode;

use phpcraft_compiler::compile;
use phpcraft_parser::{parse_file, parse_fragment};
use phpcraft_runtime::{ErrorKind, Value};
use phpcraft_vm::Vm;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(code) => ExitCode::from(code),
    }
}

fn run(args: &[String]) -> Result<u8, u8> {
    let mut index = 0;
    let mut lint = false;
    let mut inline_code: Option<String> = None;
    let mut script: Option<String> = None;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(0);
            }
            "-v" | "--version" => {
                println!("PHPCraft {VERSION} (targeting PHP 8.5.4)");
                return Ok(0);
            }
            "-l" => lint = true,
            "-r" => {
                index += 1;
                match args.get(index) {
                    Some(code) => {
                        inline_code = Some(code.clone());
                        // Everything after the code string belongs to the
                        // script, not to phpcraft's own options.
                        index += 1;
                        break;
                    }
                    None => {
                        eprintln!("phpcraft: option -r requires an argument");
                        return Err(1);
                    }
                }
            }
            "--" => {
                index += 1;
                if let Some(value) = args.get(index) {
                    script = Some(value.clone());
                    index += 1;
                }
                break;
            }
            other if other.starts_with('-') && other != "-" => {
                eprintln!("phpcraft: unknown option: {other}");
                return Err(1);
            }
            _ => {
                script = Some(arg.clone());
                index += 1;
                break;
            }
        }
        index += 1;
    }

    let remaining: Vec<String> = args[index.min(args.len())..].to_vec();

    let (source, name, argv, inline) = if let Some(code) = inline_code {
        let mut argv = vec!["Standard input code".to_string()];
        argv.extend(remaining);
        (
            code.into_bytes(),
            "Command line code".to_string(),
            argv,
            true,
        )
    } else if let Some(path) = script {
        let (source, display) = if path == "-" {
            let mut buffer = Vec::new();
            if std::io::stdin().read_to_end(&mut buffer).is_err() {
                eprintln!("phpcraft: failed to read stdin");
                return Err(1);
            }
            (buffer, "Standard input code".to_string())
        } else {
            match std::fs::read(&path) {
                Ok(buffer) => (buffer, path.clone()),
                Err(error) => {
                    eprintln!("phpcraft: could not open input file: {path} ({error})");
                    return Err(1);
                }
            }
        };
        let mut argv = vec![path];
        argv.extend(remaining);
        (source, display, argv, false)
    } else {
        print_help();
        return Ok(0);
    };

    let parsed = if inline {
        parse_fragment(&source)
    } else {
        parse_file(&source)
    };
    let program = match parsed {
        Ok(program) => program,
        Err(error) => {
            eprintln!(
                "PHP Parse error:  {} in {} on line {}",
                error.message, name, error.line
            );
            return Err(255);
        }
    };

    if lint {
        println!("No syntax errors detected in {name}");
        return Ok(0);
    }

    let compiled = match compile(&program, &name) {
        Ok(compiled) => compiled,
        Err(error) => {
            if error.line > 0 {
                eprintln!(
                    "PHP Fatal error:  {} in {} on line {}",
                    error.message, name, error.line
                );
            } else {
                eprintln!("PHP Fatal error:  {} in {}", error.message, name);
            }
            return Err(255);
        }
    };

    let mut vm = Vm::new();
    vm.set_global(
        "argv",
        Value::array(
            argv.iter()
                .map(|argument| Value::str(argument.as_bytes().to_vec()))
                .collect::<phpcraft_runtime::PhpArray>(),
        ),
    );
    vm.set_global("argc", Value::Int(argv.len() as i64));

    let result = vm.run_main(&compiled);

    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(vm.output());
    let _ = stdout.flush();

    let mut stderr = std::io::stderr();
    for warning in vm.warnings() {
        let _ = writeln!(stderr, "PHP Warning:  {warning} in {name}");
    }

    match result {
        Ok(value) => Ok(match value {
            Value::Int(code) => (code & 0xff) as u8,
            _ => 0,
        }),
        Err(error) => {
            if error.kind == ErrorKind::Exit {
                return Ok((error.exit_code.unwrap_or(0) & 0xff) as u8);
            }
            if let Some(Value::Object(object)) = &error.thrown {
                let object = object.borrow();
                let data = object.exception.as_ref();
                let message = data
                    .map(|data| data.message.as_str())
                    .unwrap_or(error.message.as_str());
                let file = data.map(|data| data.file.as_str()).unwrap_or(&name);
                let line = data.map(|data| data.line).unwrap_or(1);
                let _ = writeln!(
                    stderr,
                    "PHP Fatal error:  Uncaught {}: {} in {}:{}",
                    object.class_name, message, file, line
                );
                let _ = writeln!(stderr, "Stack trace:\n#0 {{main}}");
                let _ = writeln!(stderr, "  thrown in {} on line {}", file, line);
            } else {
                let _ = writeln!(
                    stderr,
                    "PHP Fatal error:  Uncaught {}: {} in {}",
                    error.kind.label(),
                    error.message,
                    name
                );
            }
            Ok(255)
        }
    }
}

fn print_help() {
    println!(
        "PHPCraft {VERSION} — a PHP-compatible interpreter written in Rust\n\
         \n\
         Usage: phpcraft [options] <file> [args...]\n\
         \n\
         Options:\n\
         \x20 -r <code>     Run PHP code without a file\n\
         \x20 -l            Syntax-check only\n\
         \x20 -v            Print version information\n\
         \x20 -h            Print this help\n\
         \x20 -             Read the script from stdin\n"
    );
}
