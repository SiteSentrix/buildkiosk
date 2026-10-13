//! `buildkiosk`: serve the web app, create a demo company, run commands from the command line.
#![forbid(unsafe_code)]

mod demo;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use buildkiosk_core::Timestamp;
use buildkiosk_engine::{Actor, all_commands};
use buildkiosk_server::App;
use buildkiosk_store::{DataDir, Tenant};
use serde_json::Value;

const USAGE: &str = "\
usage: buildkiosk <command> [options]

commands:
  serve    --data DIR [--company SLUG] [--bind ADDR]   run the web app (default bind 127.0.0.1:8080)
  demo     --data DIR                                  create a fictional demo company with sample projects
  run      --data DIR --company SLUG --as EMAIL|system COMMAND ['{json params}']
  mcp      --data DIR --company SLUG --as EMAIL        the MCP server on standard input and output (see docs/api.md)
  commands                                             list every command and its parameters
  token    --data DIR --company SLUG --as EMAIL [--name TEXT]   create an API token for POST /api/v1/commands/{id}
";

pub fn now() -> Timestamp {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| i64::try_from(d.as_millis()).unwrap_or(0)).unwrap_or(0);
    Timestamp::from_millis(ms)
}

/// `--name value` options and the remaining positional arguments.
struct Args {
    opts: Vec<(String, String)>,
    rest: Vec<String>,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, String> {
        let mut opts = Vec::new();
        let mut rest = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.strip_prefix("--") {
                Some(name) => {
                    let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
                    opts.push((name.to_string(), v.clone()));
                }
                None => rest.push(a.clone()),
            }
        }
        Ok(Args { opts, rest })
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.opts.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn need(&self, name: &str) -> Result<&str, String> {
        self.get(name).ok_or_else(|| format!("missing --{name}\n\n{USAGE}"))
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some((cmd, rest)) = args.split_first() else {
        print!("{USAGE}");
        return Ok(());
    };
    let a = Args::parse(rest)?;
    match cmd.as_str() {
        "serve" => serve(&a),
        "demo" => demo::create(a.need("data")?),
        "run" => run_command(&a),
        "commands" => {
            for c in all_commands() {
                println!("{:<24} {}{}\n    {}", c.id, c.label, if c.is_query() { " (query)" } else { "" }, c.params);
            }
            Ok(())
        }
        "token" => token(&a),
        "mcp" => mcp(&a),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn serve(a: &Args) -> Result<(), String> {
    let bind: SocketAddr = a.get("bind").unwrap_or("127.0.0.1:8080").parse().map_err(|e| format!("--bind: {e}"))?;
    let data = DataDir::open(a.need("data")?).map_err(|e| e.to_string())?;
    let app = Arc::new(App::open(data, a.get("company"))?);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(buildkiosk_server::serve(app, bind))
}

/// Loads a company and resolves `--as`.
fn open_company(a: &Args, data: &DataDir) -> Result<(Tenant, Actor), String> {
    let t = data.load(a.need("company")?).map_err(|e| e.to_string())?;
    let who = a.need("as")?;
    let actor = if who == "system" {
        Actor::System
    } else {
        let want = who.to_lowercase();
        let p = t
            .engine
            .state()
            .people
            .iter()
            .find(|p| p.email.as_deref().is_some_and(|e| e.to_lowercase() == want))
            .ok_or_else(|| format!("no person with email {who}"))?;
        Actor::Person(p.id)
    };
    Ok((t, actor))
}

fn run_command(a: &Args) -> Result<(), String> {
    let data = DataDir::open(a.need("data")?).map_err(|e| e.to_string())?;
    let (mut t, actor) = open_company(a, &data)?;
    let (id, params) = match a.rest.as_slice() {
        [id] => (id, Value::Object(Default::default())),
        [id, json] => (id, serde_json::from_str(json).map_err(|e| format!("parameters are not valid JSON: {e}"))?),
        _ => return Err(format!("expected COMMAND ['{{json params}}']\n\n{USAGE}")),
    };
    let out = t.engine.execute(&mut t.store, actor, now(), id, &params).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    Ok(())
}

fn token(a: &Args) -> Result<(), String> {
    let data = DataDir::open(a.need("data")?).map_err(|e| e.to_string())?;
    let (t, actor) = open_company(a, &data)?;
    let Actor::Person(p) = actor else { return Err("tokens belong to a person: use --as EMAIL".into()) };
    let mut creds = t.store.credentials().map_err(|e| e.to_string())?;
    let token = creds.create_token(p.raw(), a.get("name").unwrap_or("cli"), now()).map_err(|e| e.to_string())?;
    t.store.save_credentials(&creds).map_err(|e| e.to_string())?;
    println!("{token}");
    Ok(())
}

/// The MCP server: one JSON-RPC message per line on standard input, replies on standard output.
/// Logs go to standard error only.
fn mcp(a: &Args) -> Result<(), String> {
    use std::io::{BufRead, Write};
    let data = DataDir::open(a.need("data")?).map_err(|e| e.to_string())?;
    let (mut t, actor) = open_company(a, &data)?;
    if actor == Actor::System {
        return Err("the MCP server runs as a person: use --as EMAIL".into());
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("reading standard input: {e}"))?;
        if let Some(reply) = buildkiosk_mcp::handle_line(&mut t, actor, now(), &line) {
            writeln!(out, "{reply}").map_err(|e| format!("writing standard output: {e}"))?;
            out.flush().map_err(|e| format!("writing standard output: {e}"))?;
        }
    }
    Ok(())
}
