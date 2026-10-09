use std::process::ExitCode;
use std::path::Path;
use std::path::PathBuf;
use crate::api_client;
use std::env;
use std::fs;
use crate::mcp;
use crate::pkg;
use crate::shared::workspace::seed_inputs;
pub(crate) fn run_lex_file(path: &Path) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    };
    match hale_syntax::lex(&source) {
        Ok(tokens) => {
            for t in &tokens {
                let (line, col) = t.span.line_col(&source);
                println!("{:>4}:{:<3} {:?}", line, col, t.kind);
            }
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d.render(&source));
            }
            ExitCode::from(1)
        }
    }
}

pub(crate) fn run_parse_file(path: &Path) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    };
    match hale_syntax::parse_source(&source) {
        Ok(prog) => {
            println!("{:#?}", prog);
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d.render(&source));
            }
            ExitCode::from(1)
        }
    }
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_version() -> ExitCode {
    // The first line is the version and nothing else: the DNA
    // fixtures, the body-provisioning script and the benchmark
    // harness read `$2` of it.
    println!("hale {}", env!("CARGO_PKG_VERSION"));
    // GH #726: the DNA source a binary carries is not implied by
    // its version — two builds of one version can embed
    // different `dna/` source, and a fixture that edited the
    // working tree without rebuilding measures the old one. The
    // second line names what this binary embeds
    // (`hale dna --embedded-digest` prints all 64 hex digits).
    println!("embedded dna: {}", hale_dna::embedded_short());
    return ExitCode::SUCCESS;
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_targets() -> ExitCode {
    let host = hale_codegen::target::TargetSpec::host();
    for t in hale_codegen::target::TargetSpec::known() {
        let marker = if t.triple == host.triple {
            "  (host)"
        } else {
            ""
        };
        println!("{}{}\n", t.describe_from(&host), marker);
    }
    return ExitCode::SUCCESS;
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_fetch(args: &[String]) -> ExitCode {
    let root = if args.len() >= 3 {
        PathBuf::from(&args[2])
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    return match pkg::fetch(&root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hale fetch: {}", e);
            ExitCode::from(1)
        }
    };
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_inputs(args: &[String]) -> ExitCode {
    if args.len() < 3 {
        eprintln!("usage: hale inputs <seed-dir | file.hl>");
        return ExitCode::from(2);
    }
    return match seed_inputs(Path::new(&args[2])) {
        Ok(files) => {
            for f in files {
                println!("{}", f.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale inputs: {e}");
            ExitCode::from(1)
        }
    };
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_mcp_cmd(args: &[String]) -> ExitCode {
    // GH #1107, #1417: `hale mcp --app <endpoint>` serves a served
    // exposure's members as tools, read from its description.
    let rest: Vec<String> = args.iter().skip(2).cloned().collect();
    return match rest.as_slice() {
        [] => mcp::run_mcp(),
        [flag, tail @ ..] if flag == "--app" => mcp::run_mcp_app(tail),
        _ => {
            eprintln!("usage: hale mcp [--app <endpoint> [--token T]]");
            ExitCode::from(2)
        }
    };
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_api_client(cmd: &str, args: &[String]) -> ExitCode {
    let rest: Vec<String> = args.iter().skip(2).cloned().collect();
    return match cmd {
        "describe" => crate::api_drive::run_describe(&crate::api_drive::short_form("describe", &rest)),
        "call" => crate::api_drive::run_call(&crate::api_drive::short_form("call", &rest)),
        "watch" => api_client::run_watch(&rest),
        _ => api_client::run_admin(&rest),
    };
}
