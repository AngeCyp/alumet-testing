use alumet::agent::{static_plugins, AgentBuilder};

use clap::Parser;
use env_logger::Env;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    log::info!("Starting ALUMET relay collector v{VERSION}");

    // Parse command-line arguments.
    let args = Args::parse();

    // Load the collector plugin, and the CSV plugin to have an output.
    let plugins = static_plugins![plugin_relay::server::RelayServerPlugin, plugin_csv::CsvPlugin];

    // Build the collector
    let mut agent = AgentBuilder::new(plugins)
        .config_path("alumet-collector.toml")
        .allow_no_metrics()
        .build();

    // CLI option: config regeneration.
    if args.regen_config {
        agent
            .write_default_config()
            .expect("failed to (re)generate the configuration file");
        log::info!("Configuration file (re)generated.");
        return;
    }

    // Load the config.
    let mut config = agent.load_config().unwrap();

    // Override the config with CLI args, if any.
    if let Some(port) = args.port {
        config
            .plugin_config_mut("plugin-relay:server")
            .unwrap()
            .insert(String::from("port"), toml::Value::Integer(port.into()));
    }

    // Start the collector.
    let mut pipeline = agent.start(config);

    // Keep the pipeline running until the app closes.
    pipeline.wait_for_all();
    log::info!("ALUMET relay collector has stopped.");
}

/// Command line arguments.
#[derive(Parser)]
struct Args {
    /// Regenerate the configuration file and stop.
    ///
    /// If the file exists, it will be overwritten.
    #[arg(long)]
    regen_config: bool,

    /// The port to use when biding, for example `50051`.
    #[arg(long)]
    port: Option<u16>,
}
