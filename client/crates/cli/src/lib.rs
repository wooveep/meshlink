use std::{env, ffi::OsString, path::PathBuf};

use clap::Parser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandMode {
    Foreground { config: PathBuf },
    WindowsService { config: PathBuf },
}

#[derive(Debug, Parser)]
#[command(name = "meshlinkd", about = "MeshLink control plane client")]
struct ForegroundCli {
    #[arg(long, default_value = "deploy/examples/client-config.toml")]
    config: PathBuf,
}

pub fn parse() -> CommandMode {
    parse_from(env::args_os())
}

fn parse_from<I, T>(args: I) -> CommandMode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let collected = args.into_iter().map(Into::into).collect::<Vec<_>>();
    if collected.len() == 3 && collected[1] == OsString::from("/service") {
        return CommandMode::WindowsService {
            config: PathBuf::from(&collected[2]),
        };
    }

    let cli = ForegroundCli::parse_from(collected);
    CommandMode::Foreground { config: cli.config }
}

#[cfg(test)]
mod tests {
    use super::{parse_from, CommandMode};
    use std::path::PathBuf;

    #[test]
    fn parses_foreground_mode() {
        let mode = parse_from(["meshlinkd", "--config", "meshlink.toml"]);

        assert_eq!(
            mode,
            CommandMode::Foreground {
                config: PathBuf::from("meshlink.toml"),
            }
        );
    }

    #[test]
    fn parses_windows_service_mode() {
        let mode = parse_from(["meshlinkd.exe", "/service", "C:\\MeshLink\\MeshLink.conf"]);

        assert_eq!(
            mode,
            CommandMode::WindowsService {
                config: PathBuf::from("C:\\MeshLink\\MeshLink.conf"),
            }
        );
    }
}
