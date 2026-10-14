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
  serve    --data DIR [--company SLUG] [--bind ADDR] [--secure-cookies]
           [--backup-dir DIR [--backup-every-hours N] [--backup-keep N] [--backup-copy-to DIR]
            [--backup-s3-bucket B [--backup-s3-endpoint URL] [--backup-s3-region R] [--backup-s3-prefix P] [--backup-s3-virtual-host]]]
                                                       run the web app (default bind 127.0.0.1:8080); use
                                                       --secure-cookies behind an HTTPS proxy; --backup-dir takes
                                                       a backup while it runs (default every 24 hours, keeping 7);
                                                       --backup-copy-to also copies each one to a second folder;
                                                       --backup-s3-bucket uploads each one to S3-compatible storage
                                                       (keys in BUILDKIOSK_S3_ACCESS_KEY and BUILDKIOSK_S3_SECRET_KEY)
  backup   --data DIR --to DIR                         copy the data directory (server stopped) and verify the copy
  restore  --from DIR --data DIR [--replace | --company SLUG]  put a backup in place as the data directory, or put one
                                                       company of it back into the existing one (server stopped);
                                                       --replace moves an existing data directory aside first
  check    --data DIR                                  replay every company's journal and print row counts
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
                Some("secure-cookies") => opts.push(("secure-cookies".to_string(), "true".to_string())),
                Some("replace") => opts.push(("replace".to_string(), "true".to_string())),
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
        "backup" => backup(&a),
        "restore" => restore(&a),
        "check" => check(&a),
        "commands" => {
            for c in all_commands() {
                println!("{:<24} {}{}\n    {}", c.id, c.label, if c.is_query() { " (query)" } else { "" }, c.params);
            }
            Ok(())
        }
        "token" => token(&a),
        "mcp" => mcp(&a),
        "i18n-harvest" => i18n_harvest(&a),
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
    let mut app = App::open(data, a.get("company"))?.with_secure_cookies(a.get("secure-cookies").is_some());
    if let Some(dir) = a.get("backup-dir") {
        let number = |name: &str, default: u64| -> Result<u64, String> {
            a.get(name).map_or(Ok(default), |v| v.parse::<u64>().map_err(|_| format!("--{name} needs a whole number")))
        };
        let (hours, keep) = (number("backup-every-hours", 24)?, number("backup-keep", 7)?);
        app = app.with_backups(dir.into(), hours, usize::try_from(keep).unwrap_or(usize::MAX))?;
        if let Some(second) = a.get("backup-copy-to") {
            app = app.with_backup_copy(second.into())?;
        }
        if let Some(bucket) = a.get("backup-s3-bucket") {
            // The keys come from the environment so they are not on the command line for anyone to read in a process list.
            let key = |name: &str| std::env::var(name).map_err(|_| format!("set {name} in the environment to upload backups to object storage"));
            let region = a.get("backup-s3-region").unwrap_or("us-east-1");
            let endpoint = a.get("backup-s3-endpoint").map_or_else(|| format!("https://s3.{region}.amazonaws.com"), str::to_string);
            let target = buildkiosk_server::s3::S3Target::new(
                &endpoint,
                bucket,
                region,
                a.get("backup-s3-prefix").unwrap_or("buildkiosk/"),
                &key("BUILDKIOSK_S3_ACCESS_KEY")?,
                &key("BUILDKIOSK_S3_SECRET_KEY")?,
                a.get("backup-s3-virtual-host").is_none(),
            )
            .map_err(|e| format!("--backup-s3-*: {e}"))?;
            app = app.with_backup_object_store(target)?;
        }
    } else if ["backup-every-hours", "backup-keep", "backup-copy-to", "backup-s3-bucket"].iter().any(|f| a.get(f).is_some()) {
        return Err("--backup-every-hours, --backup-keep, --backup-copy-to and --backup-s3-bucket need --backup-dir".into());
    }
    let app = Arc::new(app);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(buildkiosk_server::serve(app, bind))
}

fn print_reports(reports: &[buildkiosk_store::CompanyReport]) {
    for r in reports {
        let rows: Vec<String> = r.rows.iter().map(|(t, n)| format!("{t} {n}")).collect();
        println!("{}: ok, {} journal entries; {}", r.slug, r.journal_entries, rows.join(", "));
    }
    if reports.is_empty() {
        println!("no companies");
    }
}

/// Puts a backup in place as the data directory, or one company of it back into the existing one.
fn restore(a: &Args) -> Result<(), String> {
    let (from, into) = (a.need("from")?, a.need("data")?);
    let seconds = u64::try_from(now().millis() / 1000).unwrap_or(0);
    if let Some(company) = a.get("company") {
        if a.get("replace").is_some() {
            return Err("--company already replaces that company (its old folder is kept aside); leave out --replace".into());
        }
        let report = buildkiosk_store::ops::restore_company(std::path::Path::new(from), std::path::Path::new(into), company, seconds)
            .map_err(|e| e.to_string())?;
        print_reports(&[report]);
        println!(
            "restored company {company} from {from} into {into}; its previous folder was kept next to the data directory (…before-restore-{company}-{seconds})"
        );
        return Ok(());
    }
    let report = buildkiosk_store::ops::restore(std::path::Path::new(from), std::path::Path::new(into), a.get("replace").is_some(), seconds)
        .map_err(|e| e.to_string())?;
    if let Some(aside) = &report.moved_aside {
        println!("the previous data directory was moved to {}", aside.display());
    }
    print_reports(&report.companies);
    println!("restored {from} to {into}");
    Ok(())
}

/// Copies the data directory to a new folder and proves the copy loads.
fn backup(a: &Args) -> Result<(), String> {
    let data = DataDir::open(a.need("data")?).map_err(|e| format!("{e} (stop the server before a backup)"))?;
    let to = a.need("to")?;
    let reports = data.backup_to(std::path::Path::new(to)).map_err(|e| e.to_string())?;
    print_reports(&reports);
    println!("backup written to {to}");
    Ok(())
}

/// Replays every company and reports; fails if any does not load.
fn check(a: &Args) -> Result<(), String> {
    let data = DataDir::open(a.need("data")?).map_err(|e| format!("{e} (stop the server before a check)"))?;
    print_reports(&data.check().map_err(|e| e.to_string())?);
    Ok(())
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

/// Lists the wording the language catalogs must cover (for the people who maintain the translations): builds the
/// demo company in a temporary folder, visits its pages and compares what is on them with the source files.
fn i18n_harvest(a: &Args) -> Result<(), String> {
    let out = a.get("out").unwrap_or("crates/web/i18n/en.txt");
    let sources = a.get("sources").unwrap_or("crates");
    let dir = std::env::temp_dir().join(format!("buildkiosk-harvest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir_str = dir.to_str().ok_or("the temporary folder has an odd name")?;
    demo::create(dir_str)?;
    let result = buildkiosk_server::harvest::run(&dir, std::path::Path::new(sources), std::path::Path::new(out), "ada@example.com", demo::PASSWORD);
    let _ = std::fs::remove_dir_all(&dir);
    println!("{}", result?);
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
    let mut session = buildkiosk_mcp::Session::default();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("reading standard input: {e}"))?;
        let reply = session.handle_line(&mut t, actor, now(), &line);
        // After every message, tell the client about subscribed resources that changed (this server is the data
        // directory's only writer, so a change is always one this client's own calls made).
        let notes = session.poll(&t, actor);
        for text in reply.into_iter().chain(notes) {
            writeln!(out, "{text}").map_err(|e| format!("writing standard output: {e}"))?;
        }
        out.flush().map_err(|e| format!("writing standard output: {e}"))?;
    }
    Ok(())
}
