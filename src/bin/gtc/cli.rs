use clap::{Arg, ArgAction, Command};

use crate::i18n_support::{leak_str, t, t_or};
use crate::router::passthrough_args;

pub(super) fn build_cli(locale: &str) -> Command {
    let cmd_args = passthrough_args();
    let options_heading = leak_str(t(locale, "gtc.help.options.heading").into_owned());
    let arguments_heading = leak_str(t(locale, "gtc.help.arguments.heading").into_owned());
    let commands_heading = leak_str(t(locale, "gtc.help.commands.heading").into_owned());
    let help_template = leak_str(build_help_template(locale));

    Command::new(leak_str(t(locale, "gtc.app.name").into_owned()))
        .version(env!("CARGO_PKG_VERSION"))
        .propagate_version(true)
        .about(t(locale, "gtc.app.about").into_owned())
        .help_template(help_template)
        .subcommand_help_heading(commands_heading)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .arg(
            Arg::new("help")
                .long("help")
                .short('h')
                .action(ArgAction::Help)
                .global(true)
                .help_heading(options_heading)
                .help(t(locale, "gtc.help.flag.help").into_owned()),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .short('V')
                .action(ArgAction::Version)
                .global(true)
                .help_heading(options_heading)
                .help(t(locale, "gtc.help.flag.version").into_owned()),
        )
        .arg(
            Arg::new("locale")
                .long("locale")
                .value_name("BCP47")
                .num_args(1)
                .global(true)
                .help_heading(options_heading)
                .help(t(locale, "gtc.arg.locale.help").into_owned()),
        )
        .arg(
            Arg::new("debug-router")
                .long("debug-router")
                .action(ArgAction::SetTrue)
                .global(true)
                .help_heading(options_heading)
                .help(t(locale, "gtc.arg.debug_router.help").into_owned()),
        )
        .subcommand(
            Command::new("version")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.version.about").into_owned()),
        )
        .subcommand(
            Command::new("doctor")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.doctor.about").into_owned()),
        )
        .subcommand(
            Command::new("install")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.install.about").into_owned())
                .arg(
                    Arg::new("tenant")
                        .long("tenant")
                        .value_name("TENANT")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.tenant.help").into_owned()),
                )
                .arg(
                    Arg::new("key")
                        .long("key")
                        .value_name("KEY")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.key.help").into_owned()),
                ),
        )
        .subcommand(
            Command::new("update")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.update.about").into_owned()),
        )
        .subcommand(
            Command::new("add-admin")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.add_admin.about").into_owned())
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help_heading(arguments_heading)
                        .help(t(locale, "gtc.arg.add_admin.bundle_ref.help").into_owned()),
                )
                .arg(
                    Arg::new("cn")
                        .long("cn")
                        .value_name("CLIENT_CN")
                        .required(true)
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.add_admin.cn.help").into_owned()),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .value_name("ADMIN_NAME")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.add_admin.name.help").into_owned()),
                )
                .arg(
                    Arg::new("public-key-file")
                        .long("public-key-file")
                        .value_name("PATH")
                        .required(true)
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.add_admin.public_key_file.help").into_owned()),
                ),
        )
        .subcommand(
            Command::new("remove-admin")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.remove_admin.about").into_owned())
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help_heading(arguments_heading)
                        .help(t(locale, "gtc.arg.remove_admin.bundle_ref.help").into_owned()),
                )
                .arg(
                    Arg::new("cn")
                        .long("cn")
                        .value_name("CLIENT_CN")
                        .num_args(1)
                        .conflicts_with("name")
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.remove_admin.cn.help").into_owned()),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .value_name("ADMIN_NAME")
                        .num_args(1)
                        .conflicts_with("cn")
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.remove_admin.name.help").into_owned()),
                ),
        )
        .subcommand(
            Command::new("admin")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.admin.about").into_owned())
                .subcommand(
                    Command::new("access")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("Show the current admin access plan for a deployed bundle.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        ),
                )
                .subcommand(
                    Command::new("certs")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about(
                            "Materialize admin client certificates locally for a deployed bundle.",
                        )
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        ),
                )
                .subcommand(
                    Command::new("token")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("Materialize the public admin relay token for a deployed bundle.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        ),
                )
                .subcommand(
                    Command::new("health")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("Probe the deployed public admin relay health endpoint.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help("Local admin tunnel port for AWS."),
                        ),
                )
                .subcommand(
                    Command::new("status")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("Fetch the remote admin runtime status for a deployed bundle.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help("Local admin tunnel port for AWS."),
                        ),
                )
                .subcommand(
                    Command::new("list")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("List bundles visible through the remote admin API.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help("Local admin tunnel port for AWS."),
                        ),
                )
                .subcommand(
                    Command::new("admins")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("List admin client CNs from the remote admin API.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help("Local admin tunnel port for AWS."),
                        ),
                )
                .subcommand(
                    Command::new("stop")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("Request a remote runtime stop through the admin API.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help("Local admin tunnel port for AWS."),
                        ),
                )
                .subcommand(
                    Command::new("add-client")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("Add an allowed admin client CN through the remote admin API.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("cn")
                                .long("cn")
                                .value_name("CLIENT_CN")
                                .required(true)
                                .num_args(1)
                                .help_heading(options_heading)
                                .help("Client common name to allow."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help("Local admin tunnel port for AWS."),
                        ),
                )
                .subcommand(
                    Command::new("remove-client")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about("Remove an allowed admin client CN through the remote admin API.")
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help("Bundle path or reference."),
                        )
                        .arg(
                            Arg::new("cn")
                                .long("cn")
                                .value_name("CLIENT_CN")
                                .required(true)
                                .num_args(1)
                                .help_heading(options_heading)
                                .help("Client common name to remove."),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws", "azure", "gcp"])
                                .help_heading(options_heading)
                                .help("Deployment target provider."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FORMAT")
                                .num_args(1)
                                .default_value("text")
                                .value_parser(["text", "json", "yaml"])
                                .help_heading(options_heading)
                                .help("Render format."),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help("Local admin tunnel port for AWS."),
                        ),
                )
                .subcommand(
                    Command::new("tunnel")
                        .help_template(help_template)
                        .subcommand_help_heading(commands_heading)
                        .disable_help_flag(true)
                        .disable_version_flag(true)
                        .about(t(locale, "gtc.cmd.admin.tunnel.about").into_owned())
                        .arg(
                            Arg::new("bundle-ref")
                                .value_name("BUNDLE_REF")
                                .required(true)
                                .help_heading(arguments_heading)
                                .help(
                                    t(locale, "gtc.arg.admin.tunnel.bundle_ref.help").into_owned(),
                                ),
                        )
                        .arg(
                            Arg::new("target")
                                .long("target")
                                .value_name("PROVIDER")
                                .num_args(1)
                                .default_value("aws")
                                .value_parser(["aws"])
                                .help_heading(options_heading)
                                .help(t(locale, "gtc.arg.admin.tunnel.target.help").into_owned()),
                        )
                        .arg(
                            Arg::new("local-port")
                                .long("local-port")
                                .value_name("PORT")
                                .num_args(1)
                                .default_value("8443")
                                .help_heading(options_heading)
                                .help(
                                    t(locale, "gtc.arg.admin.tunnel.local_port.help").into_owned(),
                                ),
                        )
                        .arg(
                            Arg::new("container")
                                .long("container")
                                .value_name("NAME")
                                .num_args(1)
                                .default_value("app")
                                .help_heading(options_heading)
                                .help(
                                    t(locale, "gtc.arg.admin.tunnel.container.help").into_owned(),
                                ),
                        ),
                ),
        )
        .subcommand(
            Command::new("start")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t_or(
                    locale,
                    "gtc.cmd.start.about",
                    "Start a bundle from local or remote reference.",
                ))
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required_unless_present("extension-start-handoff")
                        .help_heading(arguments_heading)
                        .help(t_or(
                            locale,
                            "gtc.arg.bundle_ref.help",
                            "Bundle path/ref: local path, file://, oci://, repo://, store://",
                        )),
                )
                .arg(
                    Arg::new("extension-start-handoff")
                        .long("extension-start-handoff")
                        .value_name("PATH")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(
                            "Path to a normalized extension start handoff JSON document.",
                        ),
                )
                .arg(
                    Arg::new("deploy-bundle-source")
                        .long("deploy-bundle-source")
                        .value_name("BUNDLE_SOURCE")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(t(locale, "gtc.arg.deploy_bundle_source.help").into_owned()),
                )
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("stop")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t_or(
                    locale,
                    "gtc.cmd.stop.about",
                    "Stop a bundle runtime or destroy a deployed environment.",
                ))
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help_heading(arguments_heading)
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
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.dev.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("op")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.op.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("wizard")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.wizard.about").into_owned())
                .arg(
                    Arg::new("extensions")
                        .long("extensions")
                        .value_name("ID[,ID...]")
                        .num_args(1..)
                        .help_heading(options_heading)
                        .help(
                            "Extension ids to launch through the shared extension wizard mechanism.",
                        ),
                )
                .arg(
                    Arg::new("extension-registry")
                        .long("extension-registry")
                        .value_name("PATH")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(
                            "Path to an extension registry JSON file used to resolve --extensions.",
                        ),
                )
                .arg(
                    Arg::new("emit-extension-handoff")
                        .long("emit-extension-handoff")
                        .value_name("PATH")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(
                            "Write a normalized multi-extension launcher handoff JSON document.",
                        ),
                )
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("setup")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.cmd.setup.about").into_owned())
                .arg(
                    Arg::new("extension-setup-handoff")
                        .long("extension-setup-handoff")
                        .value_name("PATH")
                        .num_args(1)
                        .help_heading(options_heading)
                        .help(
                            "Path to a normalized extension setup handoff JSON document.",
                        ),
                )
                .arg(cmd_args),
        )
        .subcommand(
            Command::new("help")
                .help_template(help_template)
                .subcommand_help_heading(commands_heading)
                .disable_help_flag(true)
                .disable_version_flag(true)
                .about(t(locale, "gtc.help.subcommand.about").into_owned())
                .arg(
                    Arg::new("command")
                        .value_name("COMMAND")
                        .num_args(0..)
                        .help_heading(arguments_heading)
                        .help(t(locale, "gtc.help.subcommand.arg.command.help").into_owned()),
                ),
        )
}

fn build_help_template(locale: &str) -> String {
    format!(
        "{{before-help}}{{about-with-newline}}{} {{usage}}\n\n{{all-args}}{{after-help}}",
        t(locale, "gtc.help.usage.heading")
    )
}
