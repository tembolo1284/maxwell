//! Full journal integrity validation.

use std::io::Write;
use std::time::Instant;

use maxwell_replay::JournalReader;

use crate::config::JournalOptions;
use crate::error::CliResult;

use crate::output::{events_per_second, optional_number, print_journal_header};

pub fn run<W: Write>(options: JournalOptions, output: &mut W) -> CliResult<()> {
    let mut reader = JournalReader::open(&options.path, options.reader_config())?;

    let started = Instant::now();

    let mut first_sequence = None;
    let mut last_sequence = None;

    let mut first_timestamp = None;
    let mut last_timestamp = None;

    while let Some(record) = reader.next_record()? {
        let sequence = record.sequence();
        let timestamp = record.recording_timestamp_ns;

        first_sequence.get_or_insert(sequence);
        last_sequence = Some(sequence);

        first_timestamp.get_or_insert(timestamp);
        last_timestamp = Some(timestamp);
    }

    let elapsed = started.elapsed();

    writeln!(output, "Journal: {}", options.path.display(),)?;

    writeln!(output)?;

    print_journal_header(output, reader.header())?;

    writeln!(output)?;
    writeln!(output, "Validation results")?;
    writeln!(output, "------------------")?;

    writeln!(output, "Records validated: {}", reader.records_read(),)?;

    writeln!(output, "Bytes validated:   {}", reader.offset(),)?;

    writeln!(
        output,
        "First sequence:    {}",
        optional_number(first_sequence),
    )?;

    writeln!(
        output,
        "Last sequence:     {}",
        optional_number(last_sequence),
    )?;

    writeln!(
        output,
        "First time (ns):   {}",
        optional_number(first_timestamp),
    )?;

    writeln!(
        output,
        "Last time (ns):    {}",
        optional_number(last_timestamp),
    )?;

    writeln!(output, "Elapsed (seconds): {:.6}", elapsed.as_secs_f64(),)?;

    writeln!(
        output,
        "Records/second:    {:.2}",
        events_per_second(reader.records_read(), elapsed),
    )?;

    if reader.incomplete_tail() {
        writeln!(output, "Status: VALID PREFIX - INCOMPLETE TAIL",)?;
    } else {
        writeln!(output, "Status: VALID")?;
    }

    Ok(())
}
