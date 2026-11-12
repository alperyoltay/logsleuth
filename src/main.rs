//! logsleuth: find, group and count the problems in a log file.

mod parse;
mod report;

use clap::{Parser, ValueEnum};
use parse::{Level, parse_line};
use report::Analyzer;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Copy, Clone, ValueEnum)]
enum LevelArg {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl From<LevelArg> for Level {
    fn from(l: LevelArg) -> Self {
        match l {
            LevelArg::Debug => Level::Debug,
            LevelArg::Info => Level::Info,
            LevelArg::Warn => Level::Warn,
            LevelArg::Error => Level::Error,
            LevelArg::Fatal => Level::Fatal,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
enum Format {
    Text,
    Json,
}

/// Find, group and count the problems in log files.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Log files to read ("-" or nothing = stdin)
    files: Vec<PathBuf>,

    /// Lowest level to group and show
    #[arg(short, long, value_enum, default_value = "warn")]
    level: LevelArg,

    /// Show at most this many groups
    #[arg(short = 'n', long, default_value_t = 10)]
    top: usize,

    /// Output format
    #[arg(short, long, value_enum, default_value = "text")]
    format: Format,

    /// Exit with code 1 when more than N lines are ERROR or worse (for CI)
    #[arg(long, value_name = "N")]
    fail_over: Option<usize>,
}

fn open(path: &PathBuf) -> io::Result<Box<dyn Read>> {
    if path.as_os_str() == "-" {
        Ok(Box::new(io::stdin()))
    } else {
        Ok(Box::new(File::open(path)?))
    }
}

fn run(cli: Cli) -> io::Result<ExitCode> {
    let mut analyzer = Analyzer::new(cli.level.into());
    let sources = if cli.files.is_empty() {
        vec![PathBuf::from("-")]
    } else {
        cli.files
    };

    let mut line_no = 0;
    for path in &sources {
        let reader = BufReader::new(
            open(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?,
        );
        for line in reader.split(b'\n') {
            line_no += 1;
            // Logs are not always valid UTF-8; don't stop on a bad byte.
            let text = String::from_utf8_lossy(&line?)
                .trim_end_matches('\r')
                .to_string();
            if text.trim().is_empty() {
                continue;
            }
            analyzer.add(line_no, &text, parse_line(&text));
        }
    }

    let report = analyzer.finish(cli.top);
    match cli.format {
        Format::Text => print!("{}", report.to_text()),
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report serializes")
        ),
    }

    if let Some(limit) = cli.fail_over {
        let errors = report.count_at_least(Level::Error);
        if errors > limit {
            eprintln!("logsleuth: {errors} error lines exceed the limit of {limit}");
            return Ok(ExitCode::from(1));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("logsleuth: {e}");
            ExitCode::from(2)
        }
    }
}
