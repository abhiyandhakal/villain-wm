use std::process::ExitCode;

use villain_ipc::{Client, DispatchRequest, Query, Request, Response, WindowId};

fn usage() -> &'static str {
    "usage:\n  villainctl dispatch close\n  villainctl dispatch minimize\n  villainctl dispatch restore-minimized\n  villainctl dispatch workspace <1-10>\n  villainctl dispatch focus-window <id>\n  villainctl dispatch restore-window <id>\n  villainctl dispatch exec <program> [args...]\n  villainctl windows\n  villainctl workspaces\n  villainctl active-window\n  villainctl active-workspace\n  villainctl version"
}

fn parse_number<T: std::str::FromStr>(value: Option<String>, name: &str) -> Result<T, String> {
    value
        .ok_or_else(|| format!("missing {name}"))?
        .parse()
        .map_err(|_| format!("invalid {name}"))
}

fn parse_request(mut args: impl Iterator<Item = String>) -> Result<Request, String> {
    match args.next().as_deref() {
        Some("dispatch") => match args.next().as_deref() {
            Some("close") => Ok(Request::Dispatch(DispatchRequest::CloseFocused)),
            Some("minimize") => Ok(Request::Dispatch(DispatchRequest::MinimizeFocused)),
            Some("restore-minimized") => {
                Ok(Request::Dispatch(DispatchRequest::RestoreLastMinimized))
            }
            Some("workspace") => Ok(Request::Dispatch(DispatchRequest::FocusWorkspace {
                workspace: parse_number(args.next(), "workspace")?,
            })),
            Some("focus-window") => Ok(Request::Dispatch(DispatchRequest::FocusWindow {
                window: WindowId(parse_number(args.next(), "window id")?),
            })),
            Some("restore-window") => Ok(Request::Dispatch(DispatchRequest::RestoreWindow {
                window: WindowId(parse_number(args.next(), "window id")?),
            })),
            Some("exec") => {
                let argv: Vec<_> = args.collect();
                if argv.is_empty() {
                    Err("dispatch exec requires a program".into())
                } else {
                    Ok(Request::Dispatch(DispatchRequest::Spawn { argv }))
                }
            }
            Some(command) => Err(format!("unknown dispatch command: {command}")),
            None => Err("missing dispatch command".into()),
        },
        Some("windows") => Ok(Request::Query(Query::Windows)),
        Some("workspaces") => Ok(Request::Query(Query::Workspaces)),
        Some("active-window") => Ok(Request::Query(Query::ActiveWindow)),
        Some("active-workspace") => Ok(Request::Query(Query::ActiveWorkspace)),
        Some("version") => Ok(Request::Query(Query::Version)),
        Some(command) => Err(format!("unknown command: {command}")),
        None => Err("missing command".into()),
    }
}

fn print_response(response: Response) -> Result<(), String> {
    match response {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(message),
        response => {
            println!(
                "{}",
                serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?
            );
            Ok(())
        }
    }
}

fn run() -> Result<(), String> {
    let request = parse_request(std::env::args().skip(1))?;
    let mut client = Client::connect().map_err(|error| error.to_string())?;
    let response = client
        .request(&request)
        .map_err(|error| error.to_string())?;
    print_response(response)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("villainctl: {error}\n\n{}", usage());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_milestone_commands() {
        assert_eq!(
            parse_request(
                ["dispatch", "workspace", "2"]
                    .map(str::to_owned)
                    .into_iter()
            )
            .unwrap(),
            Request::Dispatch(DispatchRequest::FocusWorkspace { workspace: 2 })
        );
        assert_eq!(
            parse_request(["dispatch", "minimize"].map(str::to_owned).into_iter()).unwrap(),
            Request::Dispatch(DispatchRequest::MinimizeFocused)
        );
        assert_eq!(
            parse_request(["windows"].map(str::to_owned).into_iter()).unwrap(),
            Request::Query(Query::Windows)
        );
    }
}
