use chrono::prelude::*;
use getopts::Options;
use regex::Regex;
use std::env;
use std::fs::File;
use std::io::{self, ErrorKind, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::process::{self, Child, Output, Stdio};
use std::time::{Duration, Instant};

#[derive(PartialEq)]
enum CmdKind {
    Single,
    Pipe,
    And,
    SemiCol,
}

struct Cmd<'a> {
    kind: CmdKind,
    cmd_line: &'a str,
}

struct CmdReturn {
    status: Option<i32>,
    signal: Option<i32>,
    stderr: Vec<u8>,
    stdout: Vec<u8>,
}

#[derive(PartialEq)]
enum Run {
    Continue,
    Abort,
}

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
    //opts.optmulti("m", "match", "Match for regex inside stdout", "EXPR");
    opts.optmulti(
        "e",
        "error-code",
        "Report will not be printed on stdout when ending with specified code",
        "CODE",
    );
    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!(f.to_string()),
    };

    let output_file = matches.opt_str("o");
    let error_codes = matches.opt_strs("e");
    //let _match_regex = matches.opt_strs("m");
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

    let start = Instant::now();
    let start_time = Local::now();
    let run = run_all_cmd(parse_cmd_line(&cmd_line))?;
    let end_time = Local::now();
    let duration = start.elapsed();
    let report = make_report(cmd_line, &run, &duration, start_time, end_time)?;

    if let Some(file) = output_file {
        let mut file = File::create(file)?;
        file.write_all(&report)?;
    }

    if (run.status != Some(0) && !error_codes.contains(&run.status.unwrap().to_string()))
        || !run.stderr.is_empty()
    {
        println!("{}", String::from_utf8_lossy(&report));
    }

    Ok(())
}

fn print_usage(program: &str, opts: &Options) {
    let brief = format!("Usage: {} [options] \"cmd <cmd_args>\"", program);
    print!("{}", opts.usage(&brief));
}

fn handle_cmd_output(cmd_return: &mut CmdReturn, output: &mut Output) {
    cmd_return.status = output.status.code();
    cmd_return.signal = output.status.signal();
    cmd_return.stderr.append(&mut output.stderr);
    cmd_return.stdout.append(&mut output.stdout);
}

fn handle_cmd_error(
    cmd_return: &mut CmdReturn,
    cmd: &str,
    error: io::Error,
) -> Result<(), io::Error> {
    match error.kind() {
        ErrorKind::NotFound => {
            let error_line = format!("nrbt: command not found: {}", cmd);
            cmd_return.status = Some(127);
            cmd_return.signal = None;
            cmd_return
                .stderr
                .append(&mut error_line.as_bytes().to_vec());
        }
        ErrorKind::PermissionDenied => {
            let error_line = format!("nrbt: permission denied: {}", cmd);
            cmd_return.status = Some(126);
            cmd_return.signal = None;
            cmd_return
                .stderr
                .append(&mut error_line.as_bytes().to_vec());
        }
        _ => return Err(error),
    }
    Ok(())
}

fn run_cmd(
    cmds: &[Cmd],
    indice_current: usize,
    cmd_return: &mut CmdReturn,
    mut child: Option<Child>,
) -> Result<(Run, Option<Child>), io::Error> {
    let cmd_current = &cmds[indice_current];
    let cmd_line = cmd_current.cmd_line;
    let mut cmd: Vec<&str> = cmd_line.split_whitespace().collect();
    let args = cmd.split_off(1);
    if cmd_current.kind == CmdKind::Pipe {
        let child_new = if let Some(child) = child {
            Command::new(cmd[0])
                .args(&args)
                .stdin(child.stdout.unwrap())
                .stdout(Stdio::piped())
                .spawn()
        } else {
            Command::new(cmd[0])
                .args(&args)
                .stdout(Stdio::piped())
                .spawn()
        };

        child = match child_new {
            Ok(child) => Some(child),
            Err(error) => {
                handle_cmd_error(cmd_return, &cmd[0], error)?;
                None
            }
        };
    } else {
        let output = Command::new(cmd[0]).args(&args).output();
        if indice_current > 0 {
            let cmd_last = &cmds[indice_current - 1];
            match cmd_last.kind {
                CmdKind::SemiCol => {
                    match output {
                        Ok(mut output) => handle_cmd_output(cmd_return, &mut output),
                        Err(error) => handle_cmd_error(cmd_return, &cmd[0], error)?,
                    };
                }
                CmdKind::And => {
                    if let Some(1) = &cmd_return.status {
                        return Ok((Run::Abort, None));
                    } else {
                        match output {
                            Ok(mut output) => handle_cmd_output(cmd_return, &mut output),
                            Err(error) => handle_cmd_error(cmd_return, &cmd[0], error)?,
                        };
                    }
                }
                CmdKind::Pipe => {
                    let output = if let Some(child) = child {
                        Command::new(cmd[0])
                            .args(&args)
                            .stdin(child.stdout.unwrap())
                            .output()
                    } else {
                        Command::new(cmd[0]).args(&args).output()
                    };
                    child = None;
                    match output {
                        Ok(mut output) => handle_cmd_output(cmd_return, &mut output),
                        Err(error) => handle_cmd_error(cmd_return, &cmd[0], error)?,
                    };
                }
                _ => panic!("Not supported!"),
            }
        } else {
            match output {
                Ok(mut output) => handle_cmd_output(cmd_return, &mut output),
                Err(error) => handle_cmd_error(cmd_return, &cmd[0], error)?,
            };
        }
    }

    Ok((Run::Continue, child))
}

fn run_all_cmd(cmds: Vec<Cmd>) -> Result<CmdReturn, io::Error> {
    let mut cmd_return = CmdReturn {
        status: None,
        signal: None,
        stderr: [].to_vec(),
        stdout: [].to_vec(),
    };
    let mut child: Option<Child> = None;

    for (i, _) in cmds.iter().enumerate() {
        let (run, child_new) = run_cmd(&cmds, i, &mut cmd_return, child)?;
        child = child_new;
        if run == Run::Abort {
            break;
        }
    }

    Ok(cmd_return)
}

fn parse_cmd_line(cmd_line: &str) -> Vec<Cmd> {
    let cmd_line_re = Regex::new(r"\s*([^(&{2}|;|\|)]+)(&{2}|;|\|)?").unwrap();

    cmd_line_re
        .captures_iter(cmd_line)
        .filter_map(|cap| {
            let cmd = cap.get(1);
            let separator = cap.get(2);
            if let Some(cmd) = cmd {
                if let Some(separator) = separator {
                    match separator.as_str() {
                        "&&" => Some(Cmd {
                            kind: CmdKind::And,
                            cmd_line: cmd.as_str(),
                        }),
                        "|" => Some(Cmd {
                            kind: CmdKind::Pipe,
                            cmd_line: cmd.as_str(),
                        }),
                        ";" => Some(Cmd {
                            kind: CmdKind::SemiCol,
                            cmd_line: cmd.as_str(),
                        }),
                        _ => None,
                    }
                } else {
                    Some(Cmd {
                        kind: CmdKind::Single,
                        cmd_line: cmd.as_str(),
                    })
                }
            } else {
                None
            }
        })
        .collect()
}

fn make_report(
    cmd_line: String,
    cmd_return: &CmdReturn,
    duration: &Duration,
    start_time: DateTime<Local>,
    end_time: DateTime<Local>,
) -> Result<Vec<u8>, io::Error> {
    let mut buf: Vec<u8> = Vec::new();
    writeln!(buf, "Run of command: \"{}\"", cmd_line)?;

    match cmd_return.status {
        Some(status) => writeln!(buf, "\nExit code: {}", status)?,
        None => {
            if let Some(signal) = cmd_return.signal {
                writeln!(buf, "\nTerminated by signal: {}", signal)?
            }
        }
    }

    writeln!(buf, "\nDuration: {} seconds", duration.as_secs())?;
    writeln!(buf, "Started at: {}", start_time.to_rfc2822())?;
    writeln!(buf, "Ended at: {}", end_time.to_rfc2822())?;

    writeln!(buf, "\nStdout")?;
    writeln!(buf, "------")?;
    writeln!(buf, "{}", String::from_utf8_lossy(&cmd_return.stdout))?;

    writeln!(buf, "Stderr")?;
    writeln!(buf, "------")?;
    writeln!(buf, "{}", String::from_utf8_lossy(&cmd_return.stderr))?;

    Ok(buf)
}
