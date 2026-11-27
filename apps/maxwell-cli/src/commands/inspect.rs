//! Journal header inspection.

use std::io::Write;

use maxwell_replay::JournalReader;

use crate::config::JournalOptions;
use crate::error::CliResult;
use crate::output::print_journal_header;

pub fn run<W: Write>(options: JournalOptions, output: &mut W) -> CliResult<()> {
    let reader = JournalReader::open(&options.path, options.reader_config())?;

    let file_size = std::fs::metadata(&options.path)?.len();

    writeln!(output, "Journal: {}", options.path.display(),)?;

    writeln!(output, "File size: {file_size} bytes",)?;

    writeln!(output)?;

    print_journal_header(output, reader.header())?;

    writeln!(output)?;
    writeln!(
        output,
        "Header validated; record contents have not been scanned.",
    )?;

    Ok(())
}
