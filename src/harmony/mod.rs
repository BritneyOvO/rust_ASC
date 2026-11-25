mod handler;
mod output;

pub(crate) use handler::dispatch;

pub(crate) fn configure(command: clap::Command) -> clap::Command {
    use clap::Arg;
    let command = command.mut_subcommand("findrefs", |cmd| {
        ["constant", "integer", "code"].into_iter().fold(cmd, |cmd, name| {
            cmd.subcommand(clap::Command::new(name)
                .about("Search Ark constants or resolved instructions (ABC/HAP only).")
                .arg(Arg::new("value").required(true).allow_hyphen_values(true))
                .arg(Arg::new("output").short('o').long("output")))
        })
    });
    let command = ["getclass", "findrefs", "export"].into_iter().fold(command, |command, name| {
        command.mut_subcommand(name, |cmd| cmd.arg(Arg::new("resources").long("resources").global(true)
            .help("Resource directory or HAP for a bare ABC input.")))
    });
    let command = command.mut_subcommand("getmanifest", |cmd| cmd.arg(Arg::new("json").long("json").action(clap::ArgAction::SetTrue).help("Emit Harmony manifests as JSON.")));
    ["getclass", "export"].into_iter().fold(command, |command, name| {
        command.mut_subcommand(name, |cmd| cmd.arg(Arg::new("abc_output").long("abc-output")
            .value_parser(["source", "asm"]).default_value("source")
            .help("Output mode for Harmony records.")))
    })
}
