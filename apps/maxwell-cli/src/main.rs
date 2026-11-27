//! Maxwell command-line executable.

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;

use maxwell_cli::{execute, parse_args};

fn main() -> ExitCode {
    let command = match parse_args(std::env::args_os().skip(1)) {
        Ok(command) => command,

        Err(error) => {
            eprintln!("maxwell: {error}");
            eprintln!("Run 'maxwell help' for usage.");

            return ExitCode::from(error.exit_code());
        }
    };

    let stdout = io::stdout();
    let mut output = stdout.lock();

    match execute(command, &mut output) {
        Ok(()) => {
            if let Err(error) = output.flush() {
                eprintln!("maxwell: {error}");
                return ExitCode::FAILURE;
            }

            ExitCode::SUCCESS
        }

        Err(error) => {
            eprintln!("maxwell: {error}");

            ExitCode::from(error.exit_code())
        }
    }
}
