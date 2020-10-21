use getopts::Options;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};
use std::{env, process};

fn main() -> Result<(), io::Error> {
    let args: Vec<_> = env::args().collect();
    let program_name = args[0].clone();
    let mut opts = Options::new();
    opts.optopt(
        "o",
        "output-file",
        "Write stdout and stderr in a file",
        "PATH",
    );
    opts.optmulti("m", "match", "Match for regex inside stdout", "EXPR");
    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!(f.to_string()),
    };

    let output_file = matches.opt_str("o");
    let _match_regex = matches.opt_strs("m");
    if matches.opt_present("h") {
        print_usage(&program_name, &opts);
        process::exit(0);
    }

    let cmd_line = if !matches.free.is_empty() {
        matches.free[0].clone()
    } else {
        print_usage(&program_name, &opts);
        process::exit(0);
    };

    let cmd_output = run_cmd(&cmd_line)?;
    let report = make_report(cmd_line, &cmd_output)?;

    if let Some(file) = output_file {
        let mut file = File::create(file)?;
        file.write_all(&report)?;
    }

    if cmd_output.status.code() != Some(0) || !cmd_output.stderr.is_empty() {
        println!("{}", String::from_utf8_lossy(&report));
    }

    Ok(())
}

fn print_usage(program: &str, opts: &Options) {
    let brief = format!("Usage: {} [options] \"cmd <cmd_args>\"", program);
    print!("{}", opts.usage(&brief));
}

fn run_cmd(cmd_line: &str) -> Result<Output, io::Error> {
    let mut cmd: Vec<&str> = cmd_line.split_whitespace().collect();
    let args = cmd.split_off(1);
    Command::new(cmd[0]).args(&args).output()
}

fn make_report(cmd_line: String, output: &Output) -> Result<Vec<u8>, io::Error> {
    let mut buf: Vec<u8> = Vec::new();
    writeln!(buf, "Run of command: \"{}\"", cmd_line)?;

    match output.status.code() {
        Some(code) => writeln!(buf, "\nExit code: {}", code)?,
        None => writeln!(
            buf,
            "\nTerminated by signal: {}",
            output.status.signal().unwrap()
        )?,
    };

    writeln!(buf, "\nStdout")?;
    writeln!(buf, "------")?;
    writeln!(buf, "{}", String::from_utf8_lossy(&output.stdout))?;

    writeln!(buf, "Stderr")?;
    writeln!(buf, "------")?;
    writeln!(buf, "{}", String::from_utf8_lossy(&output.stderr))?;

    Ok(buf)
}
