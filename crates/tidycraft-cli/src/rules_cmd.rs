//! `tidycraft rules` — every rule id plus a project's effective configuration,
//! which is what an agent needs to author or adjust `tidycraft.toml`.

use crate::util::{self, Format};
use crate::CliError;
use std::path::PathBuf;
use std::process::ExitCode;
use tidycraft_core::analyzer;

#[derive(clap::Args)]
pub struct RulesArgs {
    /// Project root whose config to show (default: current directory)
    pub root: Option<PathBuf>,
    /// Config file to use instead of <root>/tidycraft.toml
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Human)]
    pub format: Format,
}

pub fn run(args: RulesArgs) -> Result<ExitCode, CliError> {
    let (_root, root_str) = util::resolve_root(args.root)?;
    let (config, _doc, source) = util::load_config(&root_str, args.config.as_deref())?;

    let mut rules: Vec<(&str, &[&str])> = analyzer::RULE_ARGS.to_vec();
    rules.sort_by_key(|(id, _)| *id);

    match args.format {
        Format::Json => {
            #[derive(serde::Serialize)]
            struct RuleRow<'a> {
                id: &'a str,
                args: &'a [&'a str],
            }
            #[derive(serde::Serialize)]
            struct Report<'a> {
                schema_version: u32,
                tool: util::ToolInfo,
                config_source: Option<String>,
                rules: Vec<RuleRow<'a>>,
                config: &'a tidycraft_core::analyzer::rules::RuleConfig,
            }
            let report = Report {
                schema_version: 1,
                tool: util::tool_info(),
                config_source: source,
                rules: rules
                    .iter()
                    .map(|(id, args)| RuleRow { id, args })
                    .collect(),
                config: &config,
            };
            let json = serde_json::to_string_pretty(&report)
                .map_err(|e| CliError::Runtime(format!("serialize report: {e}")))?;
            println!("{json}");
        }
        Format::Human => {
            println!("rules ({}):", rules.len());
            for (id, _) in &rules {
                println!("  {id}");
            }
            println!();
            println!(
                "effective config (source: {}) — run `tidycraft explain <rule>` for any family:",
                source.as_deref().unwrap_or("defaults")
            );
            let toml = toml::to_string_pretty(&config)
                .map_err(|e| CliError::Runtime(format!("serialize config: {e}")))?;
            print!("{toml}");
        }
    }
    Ok(ExitCode::SUCCESS)
}
