use clap::{Arg, ArgAction, Command};

use crate::i18n_support::{leak_str, t, t_or};
use crate::router::passthrough_args;

pub(super) fn build_cli(locale: &str) -> Command {
    let cmd_args = passthrough_args();

    Command::new(leak_str(t(locale, "gtc.app.name").into_owned()))
        .version(env!("CARGO_PKG_VERSION"))
        .about(t(locale, "gtc.app.about").into_owned())
        .arg(
            Arg::new("locale")
                .long("locale")
                .value_name("BCP47")
                .num_args(1)
                .global(true)
                .help(t(locale, "gtc.arg.locale.help").into_owned()),
        )
        .arg(
            Arg::new("debug-router")
                .long("debug-router")
                .action(ArgAction::SetTrue)
                .global(true)
                .help(t(locale, "gtc.arg.debug_router.help").into_owned()),
        )
        .subcommand(Command::new("version").about(t(locale, "gtc.cmd.version.about").into_owned()))
        .subcommand(Command::new("doctor").about(t(locale, "gtc.cmd.doctor.about").into_owned()))
        .subcommand(
            Command::new("install")
                .about(t(locale, "gtc.cmd.install.about").into_owned())
                .arg(
                    Arg::new("tenant")
                        .long("tenant")
                        .value_name("TENANT")
                        .num_args(1)
                        .help(t(locale, "gtc.arg.tenant.help").into_owned()),
                )
                .arg(
                    Arg::new("key")
                        .long("key")
                        .value_name("KEY")
                        .num_args(1)
                        .help(t(locale, "gtc.arg.key.help").into_owned()),
                ),
        )
        .subcommand(Command::new("update").about(t(locale, "gtc.cmd.update.about").into_owned()))
        .subcommand(
            Command::new("add-admin")
                .about("Register an admin client certificate identity for a local bundle.")
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help("Local bundle directory to update."),
                )
                .arg(
                    Arg::new("cn")
                        .long("cn")
                        .value_name("CLIENT_CN")
                        .required(true)
                        .num_args(1)
                        .help("Client certificate Common Name allowed to access the admin API."),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .value_name("ADMIN_NAME")
                        .num_args(1)
                        .help("Optional human-readable admin label."),
                )
                .arg(
                    Arg::new("public-key-file")
                        .long("public-key-file")
                        .value_name("PATH")
                        .required(true)
                        .num_args(1)
                        .help("PEM/OpenSSH public key file for this admin."),
                ),
        )
        .subcommand(
            Command::new("remove-admin")
                .about("Remove an admin client certificate identity from a local bundle.")
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help("Local bundle directory to update."),
                )
                .arg(
                    Arg::new("cn")
                        .long("cn")
                        .value_name("CLIENT_CN")
                        .num_args(1)
                        .conflicts_with("name")
                        .help("Client certificate Common Name to remove."),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .value_name("ADMIN_NAME")
                        .num_args(1)
                        .conflicts_with("cn")
                        .help("Admin label to remove."),
                ),
        )
        .subcommand(
            Command::new("start")
                .about(t_or(
                    locale,
                    "gtc.cmd.start.about",
                    "Start a bundle from local or remote reference.",
                ))
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help(t_or(
                            locale,
                            "gtc.arg.bundle_ref.help",
                            "Bundle path/ref: local path, file://, oci://, repo://, store://",
                        )),
                )
                .arg(
                    Arg::new("deploy-bundle-source")
                        .long("deploy-bundle-source")
                        .value_name("BUNDLE_SOURCE")
                        .num_args(1)
                        .help("Override the remote bundle source passed to cloud deployers (for example https://.../bundle.gtbundle)."),
                )
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("stop")
                .about(t_or(
                    locale,
                    "gtc.cmd.stop.about",
                    "Stop a bundle runtime or destroy a deployed environment.",
                ))
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help(t_or(
                            locale,
                            "gtc.arg.bundle_ref.help",
                            "Bundle path/ref: local path, file://, oci://, repo://, store://",
                        )),
                )
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("dev")
                .about(t(locale, "gtc.cmd.dev.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("op")
                .about(t(locale, "gtc.cmd.op.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("wizard")
                .about(t(locale, "gtc.cmd.wizard.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("setup")
                .about(t(locale, "gtc.cmd.setup.about").into_owned())
                .arg(cmd_args),
        )
}
