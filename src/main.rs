use getopts::Options;
use regex::Regex;
use std::fs::File;
use std::io::{self, ErrorKind, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::time::{Duration, Instant};
use std::{env, process};

#[derive(Debug)]
enum CmdKind {
    Single,
    Pipe,
    And,
    SemiCol,
}

#[derive(Debug)]
struct Cmd<'a> {
    kind: CmdKind,
    cmd_line: &'a str,
}

#[derive(Debug)]
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

    let start = Instant::now();
    let run = run_all_cmd(parse_cmd_line(&cmd_line)).unwrap();
    let duration = start.elapsed();
    let report = make_report(cmd_line, &run, &duration)?;

    if let Some(file) = output_file {
        let mut file = File::create(file)?;
        file.write_all(&report)?;
    }

    if run.status != Some(0) || !run.stderr.is_empty() {
        println!("{}", String::from_utf8_lossy(&report));
    }

    Ok(())
}

fn print_usage(program: &str, opts: &Options) {
    let brief = format!("Usage: {} [options] \"cmd <cmd_args>\"", program);
    print!("{}", opts.usage(&brief));
}

fn run_cmd_with_precedent(
    cmd_preced: &Cmd,
    mut cmd_return: &mut CmdReturn,
    cmd_line: &str,
) -> Result<Run, io::Error> {
    let mut cmd: Vec<&str> = cmd_line.split_whitespace().collect();
    let args = cmd.split_off(1);
    match cmd_preced.kind {
        CmdKind::SemiCol => {
            let output = Command::new(cmd[0]).args(&args).output();
            match output {
                Ok(mut output) => {
                    cmd_return.status = output.status.code();
                    cmd_return.signal = output.status.signal();
                    cmd_return.stderr.append(&mut output.stderr);
                    cmd_return.stdout.append(&mut output.stdout);
                }
                Err(error) => match error.kind() {
                    ErrorKind::NotFound => {
                        cmd_return.status = Some(127);
                        cmd_return.signal = None;
                        cmd_return
                            .stderr
                            .append(&mut b"nrbt: command not found".to_vec())
                    }
                    ErrorKind::PermissionDenied => {
                        cmd_return.status = Some(126);
                        cmd_return.signal = None;
                        cmd_return
                            .stderr
                            .append(&mut b"nrbt: permission denied".to_vec())
                    }
                    _ => return Err(error),
                },
            };
        }
        CmdKind::And => {
            if let Some(1) = &cmd_return.status {
                return Ok(Run::Abort);
            } else {
                let output = Command::new(cmd[0]).args(&args).output();
                match output {
                    Ok(mut output) => {
                        cmd_return.status = output.status.code();
                        cmd_return.signal = output.status.signal();
                        cmd_return.stderr.append(&mut output.stderr);
                        cmd_return.stdout.append(&mut output.stdout);
                    }
                    Err(error) => match error.kind() {
                        ErrorKind::NotFound => {
                            cmd_return.status = Some(127);
                            cmd_return.signal = None;
                            cmd_return
                                .stderr
                                .append(&mut b"nrbt: command not found".to_vec())
                        }
                        ErrorKind::PermissionDenied => {
                            cmd_return.status = Some(126);
                            cmd_return.signal = None;
                            cmd_return
                                .stderr
                                .append(&mut b"nrbt: permission denied".to_vec())
                        }
                        _ => return Err(error),
                    },
                };
            }
        }
        _ => panic!("Not supported!"),
    }

    Ok(Run::Continue)
}

fn run_all_cmd(cmds: Vec<Cmd>) -> Result<CmdReturn, io::Error> {
    let mut cmd_return = CmdReturn {
        status: None,
        signal: None,
        stderr: [].to_vec(),
        stdout: [].to_vec(),
    };

    for (i, cmd) in cmds.iter().enumerate() {
        if i > 0 {
            let run = run_cmd_with_precedent(&cmds[i - 1], &mut cmd_return, cmd.cmd_line)?;
            if run == Run::Abort {
                break;
            }
        } else {
            let mut cmd_vec: Vec<&str> = cmd.cmd_line.split_whitespace().collect();
            let args = cmd_vec.split_off(1);
            let output = Command::new(cmd_vec[0]).args(&args).output();
            cmd_return = match output {
                Ok(output) => CmdReturn {
                    status: output.status.code(),
                    signal: output.status.signal(),
                    stderr: output.stderr,
                    stdout: output.stdout,
                },
                Err(error) => match error.kind() {
                    ErrorKind::NotFound => CmdReturn {
                        status: Some(127),
                        signal: None,
                        stderr: b"nrbt: command not found".to_vec(),
                        stdout: [].to_vec(),
                    },
                    ErrorKind::PermissionDenied => CmdReturn {
                        status: Some(126),
                        signal: None,
                        stderr: b"nrbt: permission denied".to_vec(),
                        stdout: [].to_vec(),
                    },
                    _ => return Err(error),
                },
            };
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

    writeln!(buf, "\nStdout")?;
    writeln!(buf, "------")?;
    writeln!(buf, "{}", String::from_utf8_lossy(&cmd_return.stdout))?;

    writeln!(buf, "Stderr")?;
    writeln!(buf, "------")?;
    writeln!(buf, "{}", String::from_utf8_lossy(&cmd_return.stderr))?;

    Ok(buf)
}
