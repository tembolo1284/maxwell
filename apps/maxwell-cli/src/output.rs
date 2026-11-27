//! Operational reporting and formatting.

use std::io::Write;
use std::time::Duration;

use maxwell_replay::{JournalHeader, RecordingStats, ReplayStats};

use maxwell_simulator::stats::StatsReport;

use crate::error::CliResult;

#[must_use]
pub fn optional_number(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |number| number.to_string())
}

#[must_use]
pub fn events_per_second(events: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();

    if seconds > 0.0 {
        events as f64 / seconds
    } else {
        0.0
    }
}

pub fn print_journal_header<W: Write>(output: &mut W, header: &JournalHeader) -> CliResult<()> {
    writeln!(output, "Journal metadata")?;
    writeln!(output, "----------------")?;

    writeln!(output, "Mode:              {:?}", header.mode)?;
    writeln!(output, "Recording ID:      {}", header.recording_id)?;
    writeln!(output, "Created at (ns):   {}", header.created_at_ns)?;
    writeln!(output, "Stream ID:         {}", header.feed.stream_id.raw())?;
    writeln!(
        output,
        "Session ID:        {}",
        header.feed.session_id.raw()
    )?;
    writeln!(output, "Max payload bytes: {}", header.max_payload_bytes)?;

    Ok(())
}

pub fn print_recording_stats<W: Write>(output: &mut W, stats: &RecordingStats) -> CliResult<()> {
    writeln!(output, "Recording statistics")?;
    writeln!(output, "--------------------")?;

    writeln!(output, "Records written:   {}", stats.records_written)?;
    writeln!(output, "Bytes written:     {}", stats.bytes_written)?;
    writeln!(output, "Flushes:           {}", stats.flushes)?;
    writeln!(output, "Synchronizations:  {}", stats.synchronizations)?;

    writeln!(
        output,
        "First sequence:    {}",
        optional_number(stats.first_sequence),
    )?;

    writeln!(
        output,
        "Last sequence:     {}",
        optional_number(stats.last_sequence),
    )?;

    writeln!(
        output,
        "First time (ns):   {}",
        optional_number(stats.first_timestamp_ns),
    )?;

    writeln!(
        output,
        "Last time (ns):    {}",
        optional_number(stats.last_timestamp_ns),
    )?;

    Ok(())
}

pub fn print_replay_stats<W: Write>(
    output: &mut W,
    stats: &ReplayStats,
    elapsed: Duration,
) -> CliResult<()> {
    writeln!(output, "Replay statistics")?;
    writeln!(output, "-----------------")?;

    writeln!(output, "Records read:      {}", stats.records_read)?;
    writeln!(output, "Records delivered: {}", stats.records_delivered)?;
    writeln!(output, "Records filtered:  {}", stats.records_filtered)?;

    writeln!(output, "Backpressure:      {}", stats.backpressure_attempts,)?;

    writeln!(
        output,
        "Last sequence:     {}",
        optional_number(stats.last_delivered_sequence),
    )?;

    writeln!(
        output,
        "Last time (ns):    {}",
        optional_number(stats.last_delivered_timestamp_ns),
    )?;

    writeln!(output, "Elapsed (seconds): {:.6}", elapsed.as_secs_f64(),)?;

    writeln!(
        output,
        "Records/second:    {:.2}",
        events_per_second(stats.records_delivered, elapsed),
    )?;

    Ok(())
}

pub fn print_simulation_stats<W: Write>(output: &mut W, stats: &StatsReport) -> CliResult<()> {
    writeln!(output, "Simulation statistics")?;
    writeln!(output, "---------------------")?;

    writeln!(output, "Events generated:  {}", stats.generated())?;
    writeln!(output, "Events delivered:  {}", stats.delivery.accepted)?;

    writeln!(output, "Orders added:      {}", stats.events.added)?;
    writeln!(output, "Orders modified:   {}", stats.events.modified)?;
    writeln!(output, "Orders cancelled:  {}", stats.events.cancelled)?;
    writeln!(output, "Orders deleted:    {}", stats.events.deleted)?;
    writeln!(output, "Orders executed:   {}", stats.events.executed)?;
    writeln!(output, "Other events:      {}", stats.events.other)?;

    writeln!(
        output,
        "Resting orders:    {}",
        stats.current_resting_orders,
    )?;

    writeln!(output, "Peak orders:       {}", stats.peak_resting_orders,)?;

    writeln!(output, "Delivery blocked:  {}", stats.delivery.blocked,)?;

    writeln!(output, "Generation blocked: {}", stats.generation_blocked,)?;

    writeln!(output, "Service iterations: {}", stats.service_iterations,)?;

    writeln!(
        output,
        "Drop ratio:        {:.4}%",
        stats.drop_ratio() * 100.0,
    )?;

    writeln!(
        output,
        "Duplicate ratio:   {:.4}%",
        stats.duplicate_ratio() * 100.0,
    )?;

    writeln!(
        output,
        "Reorder ratio:     {:.4}%",
        stats.reorder_ratio() * 100.0,
    )?;

    writeln!(
        output,
        "Elapsed (seconds): {:.6}",
        stats.elapsed.as_secs_f64(),
    )?;

    writeln!(
        output,
        "Generated/second:  {:.2}",
        stats.generated_per_second(),
    )?;

    writeln!(
        output,
        "Delivered/second:  {:.2}",
        stats.delivered_per_second(),
    )?;

    writeln!(output, "Fully accounted:   {}", stats.is_fully_accounted(),)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_number_is_formatted_as_dash() {
        assert_eq!(optional_number(None), "-");
    }

    #[test]
    fn present_number_is_formatted() {
        assert_eq!(optional_number(Some(42)), "42");
    }

    #[test]
    fn throughput_is_calculated() {
        assert_eq!(events_per_second(1_000, Duration::from_secs(2)), 500.0,);
    }

    #[test]
    fn zero_duration_does_not_produce_infinity() {
        assert_eq!(events_per_second(1_000, Duration::ZERO), 0.0,);
    }
}
